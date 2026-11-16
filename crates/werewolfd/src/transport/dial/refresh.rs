//! Stage52 ongoing QUIC hostname lookups. Capacity belongs to blocking
//! work, not its cancellable callers; a libc lookup cannot be aborted.

use super::interleave;
use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::{watch, Semaphore};

pub(crate) const MAX_REFRESH_JOBS: usize = 16;
type Lookup = dyn Fn(&str) -> io::Result<Vec<SocketAddr>> + Send + Sync;
#[derive(Clone)]
enum Answer {
    Resolved(Result<Arc<Vec<SocketAddr>>, Arc<io::Error>>),
    Cancelled,
}
type Flight = watch::Sender<Option<Answer>>;

pub(crate) struct Refresh {
    name: String,
    flight: Arc<Mutex<Option<Flight>>>,
    slots: Arc<Semaphore>,
    lookup: Arc<Lookup>,
}

impl Refresh {
    pub(crate) fn new(name: String) -> Self {
        static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
        Self {
            name,
            flight: Arc::new(Mutex::new(None)),
            slots: SLOTS
                .get_or_init(|| Arc::new(Semaphore::new(MAX_REFRESH_JOBS)))
                .clone(),
            lookup: Arc::new(|name| name.to_socket_addrs().map(Iterator::collect)),
        }
    }

    /// The caller's dial deadline bounds waiting, resolution and racing.
    /// Only simultaneous waiters share an answer: completed flights are
    /// replaced when the next independent dial arrives.
    pub(crate) async fn resolve(&self) -> io::Result<Arc<Vec<SocketAddr>>> {
        loop {
            let mut receiver = self.subscribe();
            loop {
                let answer = receiver.borrow_and_update().clone();
                match answer {
                    Some(Answer::Resolved(answer)) => {
                        return answer
                            .map_err(|error| io::Error::new(error.kind(), error.to_string()));
                    }
                    // A new caller can join just as the last old waiter's
                    // capacity wait is cancelled. Start fresh in that case.
                    Some(Answer::Cancelled) => break,
                    None => {}
                }
                receiver
                    .changed()
                    .await
                    .map_err(|_| io::Error::other("resolver coordinator stopped"))?;
            }
        }
    }

    fn subscribe(&self) -> watch::Receiver<Option<Answer>> {
        let mut flight = self
            .flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match flight.as_ref() {
            Some(sender) if sender.borrow().is_none() => sender.subscribe(),
            _ => {
                let (sender, receiver) = watch::channel(None);
                *flight = Some(sender.clone());
                // The coordinator owns no peer/Fang and no receiving end.
                // All cancelled waiters therefore cancel a capacity wait.
                tokio::spawn(run(
                    self.name.clone(),
                    self.slots.clone(),
                    self.lookup.clone(),
                    sender,
                    Arc::downgrade(&self.flight),
                ));
                receiver
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn with_lookup(
        name: &str,
        slots: Arc<Semaphore>,
        lookup: impl Fn(&str) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            flight: Arc::new(Mutex::new(None)),
            slots,
            lookup: Arc::new(lookup),
        }
    }
}

async fn run(
    name: String,
    slots: Arc<Semaphore>,
    lookup: Arc<Lookup>,
    sender: Flight,
    state: Weak<Mutex<Option<Flight>>>,
) {
    let permit = tokio::select! {
        // Do not submit a queued lookup after all of its callers leave,
        // even if capacity becomes available at the same time.
        biased;
        _ = sender.closed() => {
            finish(&sender, state, Answer::Cancelled);
            return;
        },
        permit = slots.acquire_owned() => match permit {
            Ok(permit) => permit,
            Err(error) => {
                finish(&sender, state, Answer::Resolved(Err(Arc::new(io::Error::other(error)))));
                return;
            }
        },
    };
    let answer = if sender.receiver_count() > 0 {
        // Never race this join against receiver closure. Started libc
        // work must finish before publishing completion for this peer.
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let addresses = interleave(lookup(&name)?);
            if addresses.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "name resolved to no address",
                ));
            }
            Ok(Arc::new(addresses))
        })
        .await
        .unwrap_or_else(|error| Err(io::Error::other(error)))
    } else {
        finish(&sender, state, Answer::Cancelled);
        return;
    };
    // Publish even if no receiver remains: a surviving peer must know it
    // can start a new lookup. An error is not cached for future dials.
    finish(&sender, state, Answer::Resolved(answer.map_err(Arc::new)));
}

fn finish(sender: &Flight, state: Weak<Mutex<Option<Flight>>>, answer: Answer) {
    sender.send_replace(Some(answer));
    // Waiters retain the answer through their receivers; the peer need not
    // retain a completed result or the coordinator after its last waiter.
    if let Some(state) = state.upgrade() {
        let mut flight = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if flight
            .as_ref()
            .is_some_and(|current| current.same_channel(sender))
        {
            *flight = None;
        }
    }
}

#[cfg(test)]
mod tests;
