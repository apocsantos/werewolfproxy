//! Bounded accept for every Fang kind (Stage42).
//!
//! A Fang's local listener accepts connections from whoever can reach its
//! bind address. Before Stage42 every Fang kind accepted without limit; a
//! flood took the peer's whole session quota, and on descriptor exhaustion
//! encrypted/plain/routed/failover Fangs stopped listening for good while the
//! QUIC Fang retried `accept` in a tight loop, logging every failure.
//!
//! [`FangAcceptor`] holds one slot per open local connection in two
//! semaphores: one per Fang and one shared by every Fang in the daemon.
//! A connection over either ceiling is accepted and reset at once, so the
//! client fails fast and never sees a session that looks complete. An accept
//! error backs off and keeps listening. Both are logged at a bounded rate.

use std::{
    net::SocketAddr,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

/// Concurrent local connections per Fang: the receiver's per-peer session
/// quota (`authority::PEER_SESSIONS`), beyond which it refuses anyway.
pub(crate) const FANG_CONNECTIONS: usize = 64;
/// Concurrent local connections across all Fangs: the receiver's global
/// session quota, well under the shipped 4096-descriptor limit.
pub(crate) const GLOBAL_FANG_CONNECTIONS: usize = 1024;
/// Pause after a failed `accept` (for example `EMFILE`) before retrying.
pub(crate) const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// At most one log line per event kind per Fang in this interval.
const LOG_INTERVAL: Duration = Duration::from_secs(10);

fn global_slots() -> Arc<Semaphore> {
    static GLOBAL: OnceLock<Arc<Semaphore>> = OnceLock::new();
    GLOBAL
        .get_or_init(|| Arc::new(Semaphore::new(GLOBAL_FANG_CONNECTIONS)))
        .clone()
}

/// Held by a Fang connection's task for as long as the session lives.
pub(crate) struct FangSlot {
    _fang: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

/// Logs the first event, then at most one line per [`LOG_INTERVAL`] with the
/// number of events suppressed in between.
struct RateLimitedLog {
    what: &'static str,
    last: Option<Instant>,
    suppressed: u64,
}

impl RateLimitedLog {
    fn new(what: &'static str) -> Self {
        Self {
            what,
            last: None,
            suppressed: 0,
        }
    }

    fn event(&mut self, fang: &str, detail: &dyn std::fmt::Display) {
        let now = Instant::now();
        if !self.due(now) {
            self.suppressed += 1;
            return;
        }
        if self.suppressed > 0 {
            eprintln!(
                "🦷 {fang} {}: {detail} ({} more since last report)",
                self.what, self.suppressed
            );
        } else {
            eprintln!("🦷 {fang} {}: {detail}", self.what);
        }
        self.last = Some(now);
        self.suppressed = 0;
    }

    fn due(&self, now: Instant) -> bool {
        self.last
            .is_none_or(|last| now.duration_since(last) >= LOG_INTERVAL)
    }
}

pub(crate) struct FangAcceptor {
    listener: TcpListener,
    fang: String,
    per_fang: Arc<Semaphore>,
    global: Arc<Semaphore>,
    accept_errors: RateLimitedLog,
    over_ceiling: RateLimitedLog,
}

impl FangAcceptor {
    pub(crate) fn new(listener: TcpListener, fang: &str) -> Self {
        Self::with_limits(
            listener,
            fang,
            Arc::new(Semaphore::new(FANG_CONNECTIONS)),
            global_slots(),
        )
    }

    pub(crate) fn with_limits(
        listener: TcpListener,
        fang: &str,
        per_fang: Arc<Semaphore>,
        global: Arc<Semaphore>,
    ) -> Self {
        Self {
            listener,
            fang: fang.to_string(),
            per_fang,
            global,
            accept_errors: RateLimitedLog::new("accept failed, retrying"),
            over_ceiling: RateLimitedLog::new("connection ceiling reached, connection reset"),
        }
    }

    /// The next local connection within both ceilings. Never returns an
    /// error: accept failures back off and retry, and connections over a
    /// ceiling are reset. Cancel-safe (the Fang's cancellation drops it).
    pub(crate) async fn accept(&mut self) -> (TcpStream, SocketAddr, FangSlot) {
        loop {
            let (stream, address) = match self.listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    self.accept_errors.event(&self.fang, &error);
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            };
            match self.slot() {
                Some(slot) => return (stream, address, slot),
                None => {
                    self.over_ceiling.event(&self.fang, &address);
                    reset(stream);
                }
            }
        }
    }

    fn slot(&self) -> Option<FangSlot> {
        let fang = self.per_fang.clone().try_acquire_owned().ok()?;
        let global = self.global.clone().try_acquire_owned().ok()?;
        Some(FangSlot {
            _fang: fang,
            _global: global,
        })
    }
}

/// Closes with RST. If arming fails the socket still closes (with FIN); the
/// client then sees an immediate EOF with no data, never a session.
fn reset(stream: TcpStream) {
    let _ = super::arm_reset_on_close(&stream);
    drop(stream);
}

#[cfg(test)]
mod tests;
