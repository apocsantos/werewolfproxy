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
//!   direction for [`IDLE_EVICT_AFTER`]. Stage54 also permits one useful
//!   own-Fang idle replacement under local peer-share pressure. Activity
//!   is recorded by [`Tracked`] on the local side of the
//!   copy paths, through a task-local set by [`FangSlot::serve`].
//! - **Per-source share.** At most [`SOURCE_SHARE`] of a Fang's slots per
//!   non-loopback source: an IPv4 address, or an IPv6 /64 (Stage44 V2), the
//!   prefix one host or link normally holds. Loopback clients all share one
//!   address and are exempt.
//!
//! Stage55 also permits one established own-Fang idle replacement at the
//! daemon-wide ceiling, below the local ceiling. Its victim budget and
//! operation gate are shared with peer reclamation; real slot cleanup
//! precedes acquisition, and its wait is charged to setup budgets.
//!
//! A connection that cannot be admitted is accepted and reset at once, so
//! the client fails fast and never sees a session that looks complete. These
//! events are logged at a bounded rate.

use std::{
    collections::HashMap,
    future::Future,
    io,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
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

#[cfg(target_os = "linux")]
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
#[cfg(target_os = "linux")]
use tokio::net::{UnixListener, UnixStream};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ShareKey {
    Ip(IpAddr),
    Uid(u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ClientIdentity {
    Tcp(SocketAddr),
    #[cfg(target_os = "linux")]
    UnixUid(u32),
}

impl std::fmt::Display for ClientIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp(address) => address.fmt(f),
            #[cfg(target_os = "linux")]
            Self::UnixUid(uid) => write!(f, "uid {uid}"),
        }
    }
}

impl ClientIdentity {
    fn share_key(self, exempt_loopback: bool) -> Option<ShareKey> {
        match self {
            Self::Tcp(address) => {
                let ip = address.ip().to_canonical();
                if exempt_loopback && ip.is_loopback() {
                    return None;
                }
                Some(ShareKey::Ip(match ip {
                    IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(
                        u128::from(v6) & !((1u128 << IPV6_SHARE_HOST_BITS) - 1),
                    )),
                    v4 => v4,
                }))
            }
            #[cfg(target_os = "linux")]
            Self::UnixUid(uid) => Some(ShareKey::Uid(uid)),
        }
    }
}

pub(crate) enum FangClientStream {
    Tcp(TcpStream),
    #[cfg(target_os = "linux")]
    Unix(UnixStream),
}

impl FangClientStream {
    fn reset(self) {
        if let Self::Tcp(stream) = self {
            reset(stream);
        }
    }

    pub(crate) fn arm_reset_on_close(&self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => super::arm_reset_on_close(stream),
            #[cfg(target_os = "linux")]
            Self::Unix(_) => Ok(()),
        }
    }

    pub(crate) fn disarm_reset_on_close(&self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => super::disarm_reset_on_close(stream),
            #[cfg(target_os = "linux")]
            Self::Unix(_) => Ok(()),
        }
    }
}

impl AsyncRead for FangClientStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.as_mut().get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(target_os = "linux")]
            Self::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for FangClientStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.as_mut().get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_write(cx, data),
            #[cfg(target_os = "linux")]
            Self::Unix(stream) => Pin::new(stream).poll_write(cx, data),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.as_mut().get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(target_os = "linux")]
            Self::Unix(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.as_mut().get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(target_os = "linux")]
            Self::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

pub(crate) struct ManagedUnixListener {
    listener: UnixListener,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl Drop for ManagedUnixListener {
    fn drop(&mut self) {
        if let Ok(metadata) = std::fs::symlink_metadata(&self.path) {
            if metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
            {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

pub(crate) enum FangListener {
    Tcp(TcpListener),
    #[cfg(target_os = "linux")]
    Unix(ManagedUnixListener),
}

impl FangListener {
    pub(crate) async fn bind(local: &str) -> io::Result<Self> {
        let Some(raw_path) = local.strip_prefix("unix:") else {
            return TcpListener::bind(local).await.map(Self::Tcp);
        };
        #[cfg(not(target_os = "linux"))]
        {
            let _ = raw_path;
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Unix Fang endpoints are supported only on Linux",
            ));
        }
        #[cfg(target_os = "linux")]
        {
            let requested_path = validate_unix_path(raw_path)?;
            let parent = requested_path.parent().ok_or_else(invalid_unix_path)?;
            let canonical_parent = std::fs::canonicalize(parent)?;
            if canonical_parent != parent {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Unix Fang socket parent path must not traverse symlinks",
                ));
            }
            let path =
                canonical_parent.join(requested_path.file_name().ok_or_else(invalid_unix_path)?);
            let parent_meta = std::fs::symlink_metadata(&canonical_parent)?;
            let parent_mode = parent_meta.mode();
            if !parent_meta.is_dir()
                || parent_meta.file_type().is_symlink()
                || parent_mode & 0o2000 == 0
                || parent_mode & 0o022 != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Unix Fang socket parent must be an operator-managed setgid directory not writable by group or others",
                ));
            }
            let listener = match UnixListener::bind(&path) {
                Ok(listener) => listener,
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!(
                            "Unix Fang socket path {} already exists; active or stale sockets are never unlinked automatically",
                            path.display()
                        ),
                    ));
                }
                Err(error) => return Err(error),
            };
            let created_meta = std::fs::symlink_metadata(&path)?;
            if !created_meta.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Unix Fang endpoint path is not a socket",
                ));
            }
            let managed = ManagedUnixListener {
                listener,
                path: path.clone(),
                device: created_meta.dev(),
                inode: created_meta.ino(),
            };
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
            let socket_meta = std::fs::symlink_metadata(&path)?;
            if !socket_meta.file_type().is_socket()
                || socket_meta.gid() != parent_meta.gid()
                || socket_meta.dev() != created_meta.dev()
                || socket_meta.ino() != created_meta.ino()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Unix Fang socket type or group does not match its managed parent",
                ));
            }
            Ok(Self::Unix(managed))
        }
    }

    async fn accept(&self) -> io::Result<(FangClientStream, ClientIdentity)> {
        match self {
            Self::Tcp(listener) => {
                let (stream, address) = listener.accept().await?;
                Ok((FangClientStream::Tcp(stream), ClientIdentity::Tcp(address)))
            }
            #[cfg(target_os = "linux")]
            Self::Unix(listener) => {
                let (stream, _) = listener.listener.accept().await?;
                let uid = stream.peer_cred()?.uid();
                Ok((FangClientStream::Unix(stream), ClientIdentity::UnixUid(uid)))
            }
        }
    }
}

fn invalid_unix_path() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Unix Fang endpoint must use unix:/absolute/path without dot components",
    )
}

#[cfg(target_os = "linux")]
fn validate_unix_path(raw: &str) -> io::Result<PathBuf> {
    let path = Path::new(raw);
    if !path.is_absolute()
        || path.file_name().is_none()
        || raw.len() > 512
        || raw.chars().any(char::is_whitespace)
        || raw.chars().any(char::is_control)
        || raw.split('/').any(|part| part == "." || part == "..")
    {
        return Err(invalid_unix_path());
    }
    Ok(path.to_path_buf())
}

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
pub(crate) const EVICT_WAIT: Duration = Duration::from_secs(1);
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
    source: Option<ShareKey>,
    evicting: bool,
    reservation: Option<session::LiveReservation>,
    established: bool,
    released: Arc<Notify>,
}

/// The open connections of one Fang (at most its ceiling, 64).
#[derive(Default)]
struct Registry {
    next: u64,
    entries: HashMap<u64, Entry>,
    peer_evictions: Option<RateLimitedLog>,
}

impl Registry {
    /// Connections `source` holds, not counting those already being evicted.
    fn source_count(&self, source: ShareKey) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.source == Some(source) && !entry.evicting)
            .count()
    }

    /// Signals the longest-idle connection idle for at least `after` (from
    /// `source` only, if given) to end. Returns how long it had been idle.
    fn evict_longest_idle(
        &mut self,
        after: Duration,
        source: Option<ShareKey>,
    ) -> Option<Duration> {
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
    _fang: Option<OwnedSemaphorePermit>,
    _global: Option<OwnedSemaphorePermit>,
    id: u64,
    registry: Arc<Mutex<Registry>>,
    activity: Arc<Activity>,
    evict: Arc<Notify>,
    session: Session,
    released: Arc<Notify>,
}

impl FangSlot {
    /// Runs one session, recording its local activity. Returns `None` if the
    /// connection was evicted to admit another; the caller then closes the
    /// local socket with a reset. The slot is released when this returns.
    pub(crate) async fn serve<F: Future>(self, session: F) -> Option<F::Output> {
        let evict = self.evict.clone();
        ACTIVITY
            .scope(
                self.activity.clone(),
                session::scope(self.session.clone(), async move {
                    tokio::select! {
                        output = session => Some(output),
                        _ = evict.notified() => None,
                    }
                }),
            )
            .await
    }
}

impl Drop for FangSlot {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.remove(self.id);
        }
        // A global waiter observes the real slot release, never just a
        // cancellation request or removal from the candidate registry.
        drop(self._fang.take());
        drop(self._global.take());
        self.released.notify_one();
    }
}

/// Logs the first event, then at most one line per [`LOG_INTERVAL`] with the
/// number of events suppressed in between.
pub(crate) struct RateLimitedLog {
    what: &'static str,
    last: Option<Instant>,
    suppressed: u64,
}

impl RateLimitedLog {
    pub(crate) fn new(what: &'static str) -> Self {
        Self {
            what,
            last: None,
            suppressed: 0,
        }
    }

    pub(crate) fn event(&mut self, fang: &str, detail: &dyn std::fmt::Display) {
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

    #[cfg(test)]
    pub(crate) fn suppressed(&self) -> u64 {
        self.suppressed
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
    fn share_key(&self, identity: ClientIdentity) -> Option<ShareKey> {
        identity.share_key(self.exempt_loopback)
    }
}

pub(crate) struct FangAcceptor {
    listener: FangListener,
    fang: String,
    per_fang: Arc<Semaphore>,
    global: Arc<Semaphore>,
    policy: Policy,
    registry: Arc<Mutex<Registry>>,
    peer_reclaim: Arc<Semaphore>,
    accept_errors: RateLimitedLog,
    over_ceiling: RateLimitedLog,
    over_share: RateLimitedLog,
    evicted: RateLimitedLog,
    global_evictions: RateLimitedLog,
}

impl FangAcceptor {
    #[cfg(test)]
    pub(crate) fn new(listener: TcpListener, fang: &str) -> Self {
        Self::new_listener(FangListener::Tcp(listener), fang)
    }

    pub(crate) fn new_listener(listener: FangListener, fang: &str) -> Self {
        if let FangListener::Tcp(tcp) = &listener {
            if let Ok(local) = tcp.local_addr() {
                if !local.ip().to_canonical().is_loopback() {
                    // S43-D5: a Fang does no authentication of its own.
                    eprintln!(
                        "🦷 {fang} listens on {local}: reachable by any host that can \
                     reach this address, not only this machine"
                    );
                }
            }
        }
        Self::with_listener_limits(
            listener,
            fang,
            Arc::new(Semaphore::new(FANG_CONNECTIONS)),
            global_slots(),
            Policy::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn with_limits(
        listener: TcpListener,
        fang: &str,
        per_fang: Arc<Semaphore>,
        global: Arc<Semaphore>,
        policy: Policy,
    ) -> Self {
        Self::with_listener_limits(FangListener::Tcp(listener), fang, per_fang, global, policy)
    }

    fn with_listener_limits(
        listener: FangListener,
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
            peer_reclaim: Arc::new(Semaphore::new(1)),
            accept_errors: RateLimitedLog::new("accept failed, retrying"),
            over_ceiling: RateLimitedLog::new("connection ceiling reached, connection reset"),
            over_share: RateLimitedLog::new("source address at its share, connection reset"),
            evicted: RateLimitedLog::new("Fang full, ended an idle connection for a new one"),
            global_evictions: RateLimitedLog::new("global ceiling full, ended an idle connection"),
        }
    }

    /// The next local connection admitted under the ceilings and shares.
    /// Never returns an error: accept failures back off and retry, and
    /// connections that cannot be admitted are reset. Cancel-safe (the
    /// Fang's cancellation drops it).
    pub(crate) async fn accept(&mut self) -> (FangClientStream, ClientIdentity, FangSlot) {
        loop {
            let (stream, identity) = match self.listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    self.accept_errors.event(&self.fang, &error);
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            };
            match self.admit(identity).await {
                Some(slot) => return (stream, identity, slot),
                None => stream.reset(),
            }
        }
    }

    async fn admit(&mut self, identity: ClientIdentity) -> Option<FangSlot> {
        let source = self.policy.share_key(identity);
        let mut evicted = None;
        if let Some(source) = source {
            if self.registry().source_count(source) >= self.policy.source_share {
                // At its share: only the source's own idle connection can
                // make room for it.
                evicted = self
                    .registry()
                    .evict_longest_idle(self.policy.idle_evict_after, Some(source));
                let Some(idle) = evicted else {
                    self.over_share.event(&self.fang, &identity);
                    return None;
                };
                self.log_eviction(idle, identity);
            }
        }
        let fang = match self.per_fang.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => self.evict_for(identity, &mut evicted).await?,
        };
        let Some((global, global_started)) = self.acquire_global(evicted.is_some()).await else {
            self.over_ceiling.event(&self.fang, &identity);
            return None;
        };
        let activity = Arc::new(Activity::new());
        let evict = Arc::new(Notify::new());
        let released = Arc::new(Notify::new());
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
                reservation: None,
                established: false,
                released: released.clone(),
            },
        );
        drop(registry);
        let mut session = Session::new(
            id,
            self.registry.clone(),
            self.peer_reclaim.clone(),
            self.fang.clone(),
            self.policy.idle_evict_after,
            evicted.is_some() || global_started.is_some(),
        );
        if let Some(started) = global_started {
            session.charge_global_wait(started);
        }
        Some(FangSlot {
            _fang: Some(fang),
            _global: Some(global),
            id,
            registry: self.registry.clone(),
            activity,
            evict,
            session,
            released,
        })
    }

    /// The Fang is full: end its longest-idle connection, if one is idle
    /// long enough (unless the share check already ended one, `evicted`),
    /// and take the slot it releases.
    async fn evict_for(
        &mut self,
        identity: ClientIdentity,
        evicted: &mut Option<Duration>,
    ) -> Option<OwnedSemaphorePermit> {
        if evicted.is_none() {
            let Some(idle) = self
                .registry()
                .evict_longest_idle(self.policy.idle_evict_after, None)
            else {
                self.over_ceiling.event(&self.fang, &identity);
                return None;
            };
            self.log_eviction(idle, identity);
            *evicted = Some(idle);
        }
        match tokio::time::timeout(EVICT_WAIT, self.per_fang.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Some(permit),
            _ => {
                self.over_ceiling.event(&self.fang, &identity);
                None
            }
        }
    }

    fn log_eviction(&mut self, idle: Duration, identity: ClientIdentity) {
        self.evicted.event(
            &self.fang,
            &format_args!("idle {} s, admitting {identity}", idle.as_secs()),
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

mod global;
mod session;
pub(crate) use session::{
    commit_current, dial_deadline, plain_connected_current, selection_deadline, Registration,
    Session,
};
