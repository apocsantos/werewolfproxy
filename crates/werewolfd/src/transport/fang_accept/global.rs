//! Stage55 global pressure: one established own-Fang victim, real release.
use super::*;
use tokio::sync::TryAcquireError;

impl Registry {
    fn global_victim(
        &mut self,
        after: Duration,
        deadline: Instant,
        pressured: impl Fn() -> bool,
    ) -> Option<(Duration, Arc<Notify>)> {
        if Instant::now() >= deadline || !pressured() {
            return None;
        }
        let now = Instant::now();
        let id = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.established && !entry.evicting)
            .map(|(id, entry)| (*id, entry.activity.idle(now)))
            .filter(|(_, idle)| *idle >= after)
            .max_by_key(|(id, idle)| (*idle, std::cmp::Reverse(*id)))?
            .0;
        let entry = self.entries.get_mut(&id)?;
        let idle = entry.activity.idle(Instant::now());
        if idle < after || Instant::now() >= deadline || !pressured() {
            return None;
        }
        entry.evicting = true;
        entry.evict.notify_one();
        Some((idle, entry.released.clone()))
    }
}

impl FangAcceptor {
    /// No global-permit queue. Only wait for the selected victim's owned
    /// cleanup, then acquire once; a competitor may still win that permit.
    pub(super) async fn acquire_global(
        &mut self,
        eviction_used: bool,
    ) -> Option<(OwnedSemaphorePermit, Option<Instant>)> {
        match self.global.clone().try_acquire_owned() {
            Ok(permit) => return Some((permit, None)),
            Err(TryAcquireError::NoPermits) if !eviction_used => {}
            Err(_) => return None,
        }
        let began = Instant::now();
        let deadline = began + EVICT_WAIT;
        // Shared with Stage54 peer reclamation; a busy operation fails
        // immediately rather than queueing another cancellation.
        let _gate = self.peer_reclaim.clone().try_acquire_owned().ok()?;
        let (idle, released) = self.registry.lock().ok()?.global_victim(
            self.policy.idle_evict_after,
            deadline,
            || !self.global.is_closed() && self.global.available_permits() == 0,
        )?;
        self.global_evictions.event(
            &self.fang,
            &format_args!("idle {} s, one replacement", idle.as_secs()),
        );
        if tokio::time::timeout_at(deadline, released.notified())
            .await
            .is_err()
            || Instant::now() >= deadline
        {
            return None;
        }
        self.global
            .clone()
            .try_acquire_owned()
            .ok()
            .map(|permit| (permit, Some(began)))
    }
}

#[cfg(test)]
mod tests;
