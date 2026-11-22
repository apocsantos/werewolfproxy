//! Stage54: session metadata, one-victim budget and bounded peer reclamation.
//! Lock order is Fang registry -> peer coordinator. No lock crosses an await.
use super::*;
use crate::transport::peer_quota::Demand;
use std::sync::atomic::AtomicBool;

pub(super) struct LiveReservation {
    demand: Demand,
    committed: bool,
    released: Arc<Notify>,
}

#[derive(Clone)]
pub(crate) struct Session {
    id: u64,
    registry: Arc<Mutex<Registry>>,
    gate: Arc<Semaphore>,
    fang: String,
    idle_after: Duration,
    eviction_used: Arc<AtomicBool>,
    first_dial: Arc<Mutex<Option<Instant>>>,
    global_started: Option<Instant>,
    global_dial_used: Arc<AtomicBool>,
}

tokio::task_local! {
    static CURRENT: Session;
}

pub(super) async fn scope<F: Future>(session: Session, future: F) -> F::Output {
    CURRENT.scope(session, future).await
}

impl Session {
    pub(super) fn new(
        id: u64,
        registry: Arc<Mutex<Registry>>,
        gate: Arc<Semaphore>,
        fang: String,
        idle_after: Duration,
        eviction_used: bool,
    ) -> Self {
        Self {
            id,
            registry,
            gate,
            fang,
            idle_after,
            eviction_used: Arc::new(AtomicBool::new(eviction_used)),
            first_dial: Arc::default(),
            global_started: None,
            global_dial_used: Arc::default(),
        }
    }

    pub(crate) fn current() -> Option<Self> {
        CURRENT.try_with(Clone::clone).ok()
    }

    pub(super) fn charge_global_wait(&mut self, began: Instant) {
        self.global_started = Some(began);
    }

    pub(crate) fn attach(&self, demand: Demand) -> io::Result<Registration> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| io::Error::other("Fang registry unavailable"))?;
        let entry = registry
            .entries
            .get_mut(&self.id)
            .ok_or_else(|| io::Error::other("Fang session ended"))?;
        if entry.reservation.is_some() {
            return Err(io::Error::other("Fang session already owns a reservation"));
        }
        let released = Arc::new(Notify::new());
        entry.reservation = Some(LiveReservation {
            demand,
            committed: false,
            released: released.clone(),
        });
        Ok(Registration {
            session: self.clone(),
            released,
        })
    }

    /// Select only a useful committed victim from this exact Fang. The
    /// caller checks the candidate's actual demand under the coordinator.
    pub(crate) fn reclaim(
        &self,
        useful: impl Fn(&Demand) -> bool,
        deadline: Instant,
    ) -> Option<Reclamation> {
        if Instant::now() >= deadline || self.eviction_used.load(Ordering::Relaxed) {
            return None;
        }
        let gate = self.gate.clone().try_acquire_owned().ok()?;
        let mut registry = self.registry.lock().ok()?;
        let now = Instant::now();
        let victim = registry
            .entries
            .iter()
            .filter(|(id, entry)| **id != self.id && !entry.evicting)
            .filter_map(|(id, entry)| {
                let held = entry.reservation.as_ref()?;
                let idle = entry.activity.idle(now);
                (held.committed && idle >= self.idle_after && useful(&held.demand))
                    .then_some((*id, idle))
            })
            .max_by_key(|(id, idle)| (*idle, std::cmp::Reverse(*id)))?
            .0;
        let entry = registry.entries.get_mut(&victim)?;
        let held = entry.reservation.as_ref()?;
        let idle = entry.activity.idle(Instant::now());
        if idle < self.idle_after
            || !useful(&held.demand)
            || Instant::now() >= deadline
            || self
                .eviction_used
                .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            return None;
        }
        entry.evicting = true;
        let released = held.released.clone();
        entry.evict.notify_one();
        registry
            .peer_evictions
            .get_or_insert_with(|| RateLimitedLog::new("peer share full, ended an idle connection"))
            .event(
                &self.fang,
                &format_args!("idle {} s, one replacement", idle.as_secs()),
            );
        Some(Reclamation {
            released,
            _gate: gate,
        })
    }

    pub(crate) fn charge_first_dial(&self, deadline: Instant) -> io::Result<()> {
        *self
            .first_dial
            .lock()
            .map_err(|_| io::Error::other("dial budget unavailable"))? = Some(deadline);
        Ok(())
    }

    pub(crate) fn clear_first_dial(&self) -> io::Result<()> {
        *self
            .first_dial
            .lock()
            .map_err(|_| io::Error::other("dial budget unavailable"))? = None;
        Ok(())
    }
}

pub(crate) struct Reclamation {
    released: Arc<Notify>,
    _gate: OwnedSemaphorePermit,
}

impl Reclamation {
    pub(crate) async fn wait(&self, deadline: Instant) -> bool {
        tokio::time::timeout_at(deadline, self.released.notified())
            .await
            .is_ok()
    }
}

pub(crate) struct Registration {
    session: Session,
    released: Arc<Notify>,
}

impl Registration {
    /// Remove the candidate before releasing its real quota. Holding the
    /// registry through `release` prevents simulation against stale usage.
    pub(crate) fn release(&self, release: impl FnOnce()) {
        let mut registry = self
            .session
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = registry.entries.get_mut(&self.session.id) {
            entry.reservation = None;
            entry.established = false;
        }
        release();
        drop(registry);
        self.released.notify_one();
    }
}

/// Only called after the final destination's ACK verifies, never at a
/// relay's outer ACK or at the start of a generic framed-copy pump.
pub(crate) fn commit_current() {
    let _ = CURRENT.try_with(|session| {
        if let Ok(mut registry) = session.registry.lock() {
            if let Some(entry) = registry.entries.get_mut(&session.id) {
                if let Some(held) = entry.reservation.as_mut() {
                    held.committed = true;
                    entry.established = true;
                }
            }
        }
    });
}

/// Plain TCP has no authenticated peer reservation. Only its successful
/// target connect establishes eligibility; an absent reservation alone does not.
pub(crate) fn plain_connected_current() {
    let _ = CURRENT.try_with(|session| {
        if let Ok(mut registry) = session.registry.lock() {
            if let Some(entry) = registry.entries.get_mut(&session.id) {
                if entry.reservation.is_none() {
                    entry.established = true;
                }
            }
        }
    });
}

/// Only a newcomer that actually reclaimed global capacity carries an
/// earlier selection start. Ordinary paths keep their existing deadlines.
pub(crate) fn selection_deadline(window: Duration) -> Instant {
    let normal = Instant::now() + window;
    CURRENT
        .try_with(|session| {
            session
                .global_started
                .map_or(normal, |began| normal.min(began + window))
        })
        .unwrap_or(normal)
}

/// Charge reclamation time to the first dial, not a fresh five-second
/// window afterward. Stage32's existing TCP setup retry keeps its own
/// later window; a subsequent provider starts its own attempt normally.
pub(crate) fn dial_deadline(window: Duration) -> Instant {
    let normal = Instant::now() + window;
    CURRENT
        .try_with(|session| {
            let global = if !session.global_dial_used.swap(true, Ordering::Relaxed) {
                session
                    .global_started
                    .map_or(normal, |began| normal.min(began + window))
            } else {
                normal
            };
            match session.first_dial.lock() {
                Ok(mut deadline) => deadline.take().map_or(global, |end| end.min(global)),
                Err(_) => Instant::now(),
            }
        })
        .unwrap_or(normal)
}

#[cfg(test)]
mod tests;
