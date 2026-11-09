use super::*;
use tokio::{io::AsyncReadExt, sync::mpsc, time::timeout};

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
    ));
    let mut accepted_b = serve(FangAcceptor::with_limits(
        listener_b,
        "fang-b",
        Arc::new(Semaphore::new(10)),
        global.clone(),
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
