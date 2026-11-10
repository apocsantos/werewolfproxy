use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
    time::{sleep, timeout},
};

/// Runs `acceptor` in a task that hands each accepted connection and its slot
/// to the test, which then decides when the slot is released.
fn serve(mut acceptor: FangAcceptor) -> mpsc::UnboundedReceiver<(TcpStream, FangSlot)> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let (stream, _, slot) = acceptor.accept().await;
            if tx.send((stream, slot)).is_err() {
                return;
            }
        }
    });
    rx
}

async fn listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    (listener, address)
}

/// A client connection the acceptor rejected sees a reset (or, if arming
/// failed, an immediate EOF): never data, never a live session.
async fn rejected(client: &mut TcpStream) -> bool {
    let mut byte = [0u8; 1];
    match timeout(Duration::from_secs(2), client.read(&mut byte)).await {
        Ok(Ok(0)) => true,
        Ok(Err(error)) => error.kind() == io::ErrorKind::ConnectionReset,
        _ => false,
    }
}

async fn still_open(client: &mut TcpStream) -> bool {
    let mut byte = [0u8; 1];
    timeout(Duration::from_millis(300), client.read(&mut byte))
        .await
        .is_err()
}

use std::io;

#[tokio::test]
async fn per_fang_ceiling_resets_excess_and_frees_on_release() {
    let (listener, address) = listener().await;
    let acceptor = FangAcceptor::with_limits(
        listener,
        "test-fang",
        Arc::new(Semaphore::new(2)),
        Arc::new(Semaphore::new(100)),
        Policy::default(),
    );
    let mut accepted = serve(acceptor);

    let mut first = TcpStream::connect(address).await.unwrap();
    let mut second = TcpStream::connect(address).await.unwrap();
    let held_first = accepted.recv().await.unwrap();
    let _held_second = accepted.recv().await.unwrap();

    // Over the ceiling: reset at once, never handed to a session.
    let mut third = TcpStream::connect(address).await.unwrap();
    assert!(rejected(&mut third).await, "third connection was not reset");
    assert!(
        timeout(Duration::from_millis(200), accepted.recv())
            .await
            .is_err(),
        "a connection over the ceiling reached a session"
    );
    assert!(still_open(&mut first).await && still_open(&mut second).await);

    // Releasing one slot admits the next connection.
    drop(held_first);
    let _fourth = TcpStream::connect(address).await.unwrap();
    assert!(
        timeout(Duration::from_secs(2), accepted.recv())
            .await
            .unwrap()
            .is_some(),
        "the next connection was not accepted after a slot was released"
    );
}

#[tokio::test]
async fn global_ceiling_is_shared_across_fangs() {
    let global = Arc::new(Semaphore::new(1));
    let (listener_a, address_a) = listener().await;
    let (listener_b, address_b) = listener().await;
    let mut accepted_a = serve(FangAcceptor::with_limits(
        listener_a,
        "fang-a",
        Arc::new(Semaphore::new(10)),
        global.clone(),
        Policy::default(),
    ));
    let mut accepted_b = serve(FangAcceptor::with_limits(
        listener_b,
        "fang-b",
        Arc::new(Semaphore::new(10)),
        global.clone(),
        Policy::default(),
    ));

    let _a = TcpStream::connect(address_a).await.unwrap();
    let held = accepted_a.recv().await.unwrap();

    // Fang B has per-Fang room, but the shared slot is taken.
    let mut b = TcpStream::connect(address_b).await.unwrap();
    assert!(
        rejected(&mut b).await,
        "global ceiling did not apply to fang B"
    );
    // A rejected global slot must not leak fang B's per-Fang permit either.
    drop(held);
    let _b2 = TcpStream::connect(address_b).await.unwrap();
    assert!(timeout(Duration::from_secs(2), accepted_b.recv())
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn production_limits_match_the_receiver_quotas() {
    assert_eq!(FANG_CONNECTIONS, 64);
    assert_eq!(GLOBAL_FANG_CONNECTIONS, 1024);
    let (listener, _) = listener().await;
    let acceptor = FangAcceptor::new(listener, "limits");
    assert_eq!(acceptor.per_fang.available_permits(), FANG_CONNECTIONS);
    assert!(Arc::ptr_eq(&acceptor.global, &global_slots()));
}

#[tokio::test]
async fn repeated_events_are_logged_at_a_bounded_rate() {
    let mut log = RateLimitedLog::new("accept failed, retrying");
    assert!(log.due(Instant::now()));
    log.event("fang", &"first");
    // A tight burst inside one interval logs nothing more.
    for _ in 0..10_000 {
        log.event("fang", &"again");
    }
    assert_eq!(log.suppressed, 10_000);
    assert!(!log.due(Instant::now()));
    // One interval later (moved back rather than waited for).
    log.last = log.last.map(|last| last - LOG_INTERVAL);
    assert!(log.due(Instant::now()));
    log.event("fang", &"after interval");
    assert_eq!(log.suppressed, 0);
}

// --- Stage43: idle eviction and the per-source share -------------------------

/// Accepts forever; each connection runs an echo session under its slot,
/// as the forwarders do, and reports whether it was evicted.
fn echo_sessions(mut acceptor: FangAcceptor) -> mpsc::UnboundedReceiver<bool> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let (mut stream, _, slot) = acceptor.accept().await;
            let tx = tx.clone();
            tokio::spawn(async move {
                let evicted = slot.serve(echo(&mut stream)).await.is_none();
                if evicted {
                    let _ = crate::transport::arm_reset_on_close(&stream);
                }
                drop(stream);
                let _ = tx.send(evicted);
            });
        }
    });
    rx
}

async fn echo(stream: &mut TcpStream) {
    let mut local = Tracked::new(stream);
    let mut buf = [0u8; 1024];
    while let Ok(n) = local.read(&mut buf).await {
        if n == 0 || local.write_all(&buf[..n]).await.is_err() {
            return;
        }
    }
}

async fn echoes(client: &mut TcpStream) -> bool {
    let mut buf = [0u8; 4];
    client.write_all(b"ping").await.is_ok()
        && matches!(
            timeout(Duration::from_secs(2), client.read_exact(&mut buf)).await,
            Ok(Ok(_))
        )
        && &buf == b"ping"
}

fn policy(idle_evict_after: Duration, source_share: usize, exempt_loopback: bool) -> Policy {
    Policy {
        idle_evict_after,
        source_share,
        exempt_loopback,
    }
}

#[tokio::test]
async fn full_fang_evicts_its_longest_idle_connection() {
    let (listener, address) = listener().await;
    let per_fang = Arc::new(Semaphore::new(2));
    let mut ended = echo_sessions(FangAcceptor::with_limits(
        listener,
        "evict",
        per_fang.clone(),
        Arc::new(Semaphore::new(100)),
        policy(Duration::from_millis(300), 100, true),
    ));

    let mut first = TcpStream::connect(address).await.unwrap();
    assert!(echoes(&mut first).await);
    sleep(Duration::from_millis(150)).await;
    let mut second = TcpStream::connect(address).await.unwrap();
    assert!(echoes(&mut second).await);
    // Both idle past the threshold; the first has been idle longest.
    sleep(Duration::from_millis(400)).await;

    let mut third = TcpStream::connect(address).await.unwrap();
    assert!(echoes(&mut third).await, "the newcomer was not admitted");
    assert!(
        rejected(&mut first).await,
        "the longest-idle connection was not reset"
    );
    assert!(
        echoes(&mut second).await,
        "a connection other than the longest-idle ended"
    );
    assert_eq!(ended.recv().await, Some(true));
}

#[tokio::test]
async fn recently_active_connections_are_never_evicted() {
    let (listener, address) = listener().await;
    let mut ended = echo_sessions(FangAcceptor::with_limits(
        listener,
        "active",
        Arc::new(Semaphore::new(2)),
        Arc::new(Semaphore::new(100)),
        policy(Duration::from_millis(300), 100, true),
    ));

    let mut first = TcpStream::connect(address).await.unwrap();
    let mut second = TcpStream::connect(address).await.unwrap();
    assert!(echoes(&mut first).await && echoes(&mut second).await);
    sleep(Duration::from_millis(400)).await;
    // Activity on both resets their idle time, either direction counts.
    assert!(echoes(&mut first).await && echoes(&mut second).await);

    let mut third = TcpStream::connect(address).await.unwrap();
    assert!(
        rejected(&mut third).await,
        "a full Fang admitted over active sessions"
    );
    assert!(echoes(&mut first).await && echoes(&mut second).await);
    assert!(timeout(Duration::from_millis(200), ended.recv())
        .await
        .is_err());
}

#[tokio::test]
async fn a_source_address_is_held_to_its_share() {
    let (listener, address) = listener().await;
    let acceptor = FangAcceptor::with_limits(
        listener,
        "share",
        Arc::new(Semaphore::new(10)),
        Arc::new(Semaphore::new(100)),
        // Loopback stands in for one LAN host here.
        policy(Duration::from_secs(60), 2, false),
    );
    let registry = acceptor.registry.clone();
    let mut ended = echo_sessions(acceptor);

    let mut first = TcpStream::connect(address).await.unwrap();
    let mut second = TcpStream::connect(address).await.unwrap();
    assert!(echoes(&mut first).await && echoes(&mut second).await);
    let mut third = TcpStream::connect(address).await.unwrap();
    assert!(
        rejected(&mut third).await,
        "a source held more than its share"
    );

    // A released slot returns to the source's share.
    drop(first);
    assert_eq!(ended.recv().await, Some(false));
    let mut fourth = TcpStream::connect(address).await.unwrap();
    assert!(echoes(&mut fourth).await);
    assert_eq!(
        registry
            .lock()
            .unwrap()
            .source_count("127.0.0.1".parse().unwrap()),
        2
    );
}

#[tokio::test]
async fn slots_and_shares_are_released_when_sessions_end() {
    let (listener, address) = listener().await;
    let per_fang = Arc::new(Semaphore::new(4));
    let global = Arc::new(Semaphore::new(4));
    let acceptor = FangAcceptor::with_limits(
        listener,
        "release",
        per_fang.clone(),
        global.clone(),
        policy(Duration::from_millis(100), 4, false),
    );
    let registry = acceptor.registry.clone();
    let mut ended = echo_sessions(acceptor);

    let mut clients = Vec::new();
    for _ in 0..4 {
        let mut client = TcpStream::connect(address).await.unwrap();
        assert!(echoes(&mut client).await);
        clients.push(client);
    }
    // The source is at its share and the Fang is full: the newcomer
    // evicts the source's own longest-idle connection, and only that one.
    sleep(Duration::from_millis(200)).await;
    let mut newcomer = TcpStream::connect(address).await.unwrap();
    assert!(echoes(&mut newcomer).await);
    assert_eq!(ended.recv().await, Some(true));
    assert!(timeout(Duration::from_millis(200), ended.recv())
        .await
        .is_err());
    clients.push(newcomer);
    drop(clients);
    for _ in 0..4 {
        assert_eq!(
            timeout(Duration::from_secs(2), ended.recv()).await.unwrap(),
            Some(false)
        );
    }

    assert!(registry.lock().unwrap().entries.is_empty());
    assert_eq!(per_fang.available_permits(), 4);
    assert_eq!(global.available_permits(), 4);
}

#[test]
fn only_non_loopback_sources_are_shared() {
    let production = Policy::default();
    for loopback in [
        "127.0.0.1:1",
        "127.0.0.2:1",
        "[::1]:1",
        "[::ffff:127.0.0.1]:1",
    ] {
        assert_eq!(
            production.share_key(loopback.parse().unwrap()),
            None,
            "{loopback}"
        );
    }
    assert_eq!(
        production.share_key("10.43.0.2:1".parse().unwrap()),
        Some("10.43.0.2".parse().unwrap())
    );
    assert_eq!(
        production.share_key("[::ffff:10.43.0.2]:1".parse().unwrap()),
        Some("10.43.0.2".parse().unwrap()),
        "an IPv4-mapped address shares with its IPv4 form"
    );
    assert_eq!(production.idle_evict_after, IDLE_EVICT_AFTER);
    assert_eq!((IDLE_EVICT_AFTER.as_secs(), SOURCE_SHARE), (60, 16));
}

/// Stage44 V2: one IPv6 host holds a /64, so IPv6 sources share by /64.
#[test]
fn ipv6_sources_share_by_slash_64() {
    let policy = Policy::default();
    let key = |a: &str| policy.share_key(a.parse().unwrap());
    let prefix = Some("fd43::".parse().unwrap());
    for same in [
        "[fd43::10]:1",
        "[fd43::13]:2",
        "[fd43::ffff:ffff:ffff:ffff]:3",
    ] {
        assert_eq!(key(same), prefix, "{same}");
    }
    assert_eq!(
        key("[fd43:0:0:1::2]:1"),
        Some("fd43:0:0:1::".parse().unwrap())
    );
    assert_ne!(
        key("[fd43:0:0:1::2]:1"),
        prefix,
        "another /64 is another source"
    );
    // IPv4 and IPv4-mapped keep their exact address.
    assert_eq!(key("10.43.0.2:1"), Some("10.43.0.2".parse().unwrap()));
    assert_eq!(
        key("[::ffff:10.43.0.2]:1"),
        Some("10.43.0.2".parse().unwrap())
    );
    assert_ne!(key("10.43.0.2:1"), key("10.43.0.3:1"));
    assert_eq!(key("[::1]:1"), None);
}

#[tokio::test]
async fn tracked_records_activity_only_inside_a_fang_session() {
    let activity = Arc::new(Activity::new());
    activity.0.store(0, Ordering::Relaxed);
    let (a, mut b) = tokio::io::duplex(64);

    // Outside a session: a pass-through.
    let mut untracked = Tracked::new(a);
    untracked.write_all(b"x").await.unwrap();
    assert_eq!(activity.0.load(Ordering::Relaxed), 0);
    let a = untracked.inner;

    ACTIVITY
        .scope(activity.clone(), async move {
            let mut tracked = Tracked::new(a);
            sleep(Duration::from_millis(5)).await;
            tracked.write_all(b"y").await.unwrap();
            let written = activity.0.load(Ordering::Relaxed);
            assert!(written > 0, "a write was not recorded");
            sleep(Duration::from_millis(5)).await;
            b.write_all(b"z").await.unwrap();
            let mut byte = [0u8; 1];
            tracked.read_exact(&mut byte).await.unwrap();
            assert!(
                activity.0.load(Ordering::Relaxed) > written,
                "a read was not recorded"
            );
        })
        .await;
}
