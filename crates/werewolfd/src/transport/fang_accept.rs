//! Bounded, fairer accept for every Fang kind (Stage42, Stage43).
//!
//! A Fang's local listener accepts connections from whoever can reach its
//! bind address. Before Stage42 every Fang kind accepted without limit; a
//! flood took the peer's whole session quota, and on descriptor exhaustion
//! encrypted/plain/routed/failover Fangs stopped listening for good while the
//! QUIC Fang retried `accept` in a tight loop, logging every failure.
//!
//! [`FangAcceptor`] holds one slot per open local connection in two
//! semaphores: one per Fang and one shared by every Fang in the daemon. An
//! accept error backs off and keeps listening.
//!
//! Stage43 adds two rules so that one client cannot keep a Fang's slots from
//! every other client:
//!
//! - **Idle eviction.** When a Fang is full, its longest-idle connection is
//!   ended to admit the newcomer, if no byte has passed on it in either
//!   direction for [`IDLE_EVICT_AFTER`]. A Fang under its ceiling never
//!   evicts. Activity is recorded by [`Tracked`] on the local side of the
//!   copy paths, through a task-local set by [`FangSlot::serve`].
//! - **Per-source share.** At most [`SOURCE_SHARE`] of a Fang's slots per
//!   non-loopback source: an IPv4 address, or an IPv6 /64 (Stage44 V2), the
//!   prefix one host or link normally holds. Loopback clients all share one
//!   address and are exempt.
//!
//! A connection that cannot be admitted is accepted and reset at once, so
//! the client fails fast and never sees a session that looks complete. These
//! events are logged at a bounded rate.

use std::{
    collections::HashMap,
    future::Future,
    io,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{Notify, OwnedSemaphorePermit, Semaphore},
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
/// A connection with no byte either way for this long may be ended to admit
/// a new one, when its Fang is full (Stage43 S43-D2).
pub(crate) const IDLE_EVICT_AFTER: Duration = Duration::from_secs(60);
/// Slots per Fang one non-loopback source address may hold (S43-D2).
pub(crate) const SOURCE_SHARE: usize = 16;
/// IPv6 sources share by /64: the low 64 bits are ignored (Stage44 V2).
const IPV6_SHARE_HOST_BITS: u32 = 64;
/// How long a newcomer waits for an evicted connection to release its slot.
const EVICT_WAIT: Duration = Duration::from_secs(1);
/// At most one log line per event kind per Fang in this interval.
const LOG_INTERVAL: Duration = Duration::from_secs(10);

fn global_slots() -> Arc<Semaphore> {
    static GLOBAL: OnceLock<Arc<Semaphore>> = OnceLock::new();
    GLOBAL
        .get_or_init(|| Arc::new(Semaphore::new(GLOBAL_FANG_CONNECTIONS)))
        .clone()
}

/// The monotonic origin for activity timestamps.
fn origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

/// When a connection last moved a byte, in milliseconds since [`origin`].
pub(crate) struct Activity(AtomicU64);

impl Activity {
    fn new() -> Self {
        let activity = Self(AtomicU64::new(0));
        activity.touch();
        activity
    }

    pub(crate) fn touch(&self) {
        let now = Instant::now().duration_since(origin()).as_millis() as u64;
        self.0.store(now, Ordering::Relaxed);
    }

    fn idle(&self, now: Instant) -> Duration {
        let now = now.duration_since(origin()).as_millis() as u64;
        Duration::from_millis(now.saturating_sub(self.0.load(Ordering::Relaxed)))
    }
}

tokio::task_local! {
    static ACTIVITY: Arc<Activity>;
}

/// Records activity on the wrapped local stream, for the Fang connection
/// whose [`FangSlot::serve`] runs the current task. Outside a Fang session
/// (the receiver, discovery, tests) it is a plain pass-through.
pub(crate) struct Tracked<T> {
    inner: T,
    activity: Option<Arc<Activity>>,
}

impl<T> Tracked<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self {
            inner,
            activity: ACTIVITY.try_with(Arc::clone).ok(),
        }
    }

    fn touch(&self) {
        if let Some(activity) = &self.activity {
            activity.touch();
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Tracked<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(poll, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            self.touch();
        }
        poll
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Tracked<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, data);
        if matches!(poll, Poll::Ready(Ok(n)) if n > 0) {
            self.touch();
        }
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

struct Entry {
    activity: Arc<Activity>,
    evict: Arc<Notify>,
    source: Option<IpAddr>,
    evicting: bool,
}

/// The open connections of one Fang (at most its ceiling, 64).
#[derive(Default)]
struct Registry {
    next: u64,
    entries: HashMap<u64, Entry>,
}

impl Registry {
    /// Connections `source` holds, not counting those already being evicted.
    fn source_count(&self, source: IpAddr) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.source == Some(source) && !entry.evicting)
            .count()
    }

    /// Signals the longest-idle connection idle for at least `after` (from
    /// `source` only, if given) to end. Returns how long it had been idle.
    fn evict_longest_idle(&mut self, after: Duration, source: Option<IpAddr>) -> Option<Duration> {
        let now = Instant::now();
        let (idle, entry) = self
            .entries
            .values_mut()
            .filter(|entry| !entry.evicting)
            .filter(|entry| source.is_none() || entry.source == source)
            .map(|entry| (entry.activity.idle(now), entry))
            .filter(|(idle, _)| *idle >= after)
            .max_by_key(|(idle, _)| *idle)?;
        entry.evicting = true;
        entry.evict.notify_one();
        Some(idle)
    }

    fn remove(&mut self, id: u64) {
        self.entries.remove(&id);
    }
}

/// Held by a Fang connection's task for as long as the session lives.
pub(crate) struct FangSlot {
    _fang: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
    id: u64,
    registry: Arc<Mutex<Registry>>,
    activity: Arc<Activity>,
    evict: Arc<Notify>,
}

impl FangSlot {
    /// Runs one session, recording its local activity. Returns `None` if the
    /// connection was evicted to admit another; the caller then closes the
    /// local socket with a reset. The slot is released when this returns.
    pub(crate) async fn serve<F: Future>(self, session: F) -> Option<F::Output> {
        let evict = self.evict.clone();
        ACTIVITY
            .scope(self.activity.clone(), async move {
                tokio::select! {
                    output = session => Some(output),
                    _ = evict.notified() => None,
                }
            })
            .await
    }
}

impl Drop for FangSlot {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.remove(self.id);
        }
    }
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

/// Admission rules; [`Policy::default`] is production.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Policy {
    pub(crate) idle_evict_after: Duration,
    pub(crate) source_share: usize,
    /// Loopback clients share one address, so they are exempt from the
    /// per-source share. Tests turn this off to exercise the share.
    pub(crate) exempt_loopback: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            idle_evict_after: IDLE_EVICT_AFTER,
            source_share: SOURCE_SHARE,
            exempt_loopback: true,
        }
    }
}

impl Policy {
    /// The source a connection's share is counted under, if any: the IPv4
    /// address (an IPv4-mapped address counts as its IPv4 form), or the /64
    /// of an IPv6 address.
    fn share_key(&self, address: SocketAddr) -> Option<IpAddr> {
        let ip = address.ip().to_canonical();
        if self.exempt_loopback && ip.is_loopback() {
            return None;
        }
        Some(match ip {
            IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(
                u128::from(v6) & !((1u128 << IPV6_SHARE_HOST_BITS) - 1),
            )),
            v4 => v4,
        })
    }
}

pub(crate) struct FangAcceptor {
    listener: TcpListener,
    fang: String,
    per_fang: Arc<Semaphore>,
    global: Arc<Semaphore>,
    policy: Policy,
    registry: Arc<Mutex<Registry>>,
    accept_errors: RateLimitedLog,
    over_ceiling: RateLimitedLog,
    over_share: RateLimitedLog,
    evicted: RateLimitedLog,
}

impl FangAcceptor {
    pub(crate) fn new(listener: TcpListener, fang: &str) -> Self {
        if let Ok(local) = listener.local_addr() {
            if !local.ip().to_canonical().is_loopback() {
                // S43-D5: a Fang does no authentication of its own.
                eprintln!(
                    "🦷 {fang} listens on {local}: reachable by any host that can \
                     reach this address, not only this machine"
                );
            }
        }
        Self::with_limits(
            listener,
            fang,
            Arc::new(Semaphore::new(FANG_CONNECTIONS)),
            global_slots(),
            Policy::default(),
        )
    }

    pub(crate) fn with_limits(
        listener: TcpListener,
        fang: &str,
        per_fang: Arc<Semaphore>,
        global: Arc<Semaphore>,
        policy: Policy,
    ) -> Self {
        Self {
            listener,
            fang: fang.to_string(),
            per_fang,
            global,
            policy,
            registry: Arc::default(),
            accept_errors: RateLimitedLog::new("accept failed, retrying"),
            over_ceiling: RateLimitedLog::new("connection ceiling reached, connection reset"),
            over_share: RateLimitedLog::new("source address at its share, connection reset"),
            evicted: RateLimitedLog::new("Fang full, ended an idle connection for a new one"),
        }
    }

    /// The next local connection admitted under the ceilings and shares.
    /// Never returns an error: accept failures back off and retry, and
    /// connections that cannot be admitted are reset. Cancel-safe (the
    /// Fang's cancellation drops it).
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
            match self.admit(address).await {
                Some(slot) => return (stream, address, slot),
                None => reset(stream),
            }
        }
    }

    async fn admit(&mut self, address: SocketAddr) -> Option<FangSlot> {
        let source = self.policy.share_key(address);
        let mut evicted = None;
        if let Some(source) = source {
            if self.registry().source_count(source) >= self.policy.source_share {
                // At its share: only the source's own idle connection can
                // make room for it.
                evicted = self
                    .registry()
                    .evict_longest_idle(self.policy.idle_evict_after, Some(source));
                let Some(idle) = evicted else {
                    self.over_share.event(&self.fang, &address);
                    return None;
                };
                self.log_eviction(idle, address);
            }
        }
        let fang = match self.per_fang.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => self.evict_for(address, evicted).await?,
        };
        let Ok(global) = self.global.clone().try_acquire_owned() else {
            self.over_ceiling.event(&self.fang, &address);
            return None;
        };
        let activity = Arc::new(Activity::new());
        let evict = Arc::new(Notify::new());
        let mut registry = self.registry();
        let id = registry.next;
        registry.next += 1;
        registry.entries.insert(
            id,
            Entry {
                activity: activity.clone(),
                evict: evict.clone(),
                source,
                evicting: false,
            },
        );
        drop(registry);
        Some(FangSlot {
            _fang: fang,
            _global: global,
            id,
            registry: self.registry.clone(),
            activity,
            evict,
        })
    }

    /// The Fang is full: end its longest-idle connection, if one is idle
    /// long enough (unless the share check already ended one, `evicted`),
    /// and take the slot it releases.
    async fn evict_for(
        &mut self,
        address: SocketAddr,
        evicted: Option<Duration>,
    ) -> Option<OwnedSemaphorePermit> {
        if evicted.is_none() {
            let Some(idle) = self
                .registry()
                .evict_longest_idle(self.policy.idle_evict_after, None)
            else {
                self.over_ceiling.event(&self.fang, &address);
                return None;
            };
            self.log_eviction(idle, address);
        }
        match tokio::time::timeout(EVICT_WAIT, self.per_fang.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Some(permit),
            _ => {
                self.over_ceiling.event(&self.fang, &address);
                None
            }
        }
    }

    fn log_eviction(&mut self, idle: Duration, address: SocketAddr) {
        self.evicted.event(
            &self.fang,
            &format_args!("idle {} s, admitting {address}", idle.as_secs()),
        );
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
