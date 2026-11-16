use super::*;
use crate::transport::dial::{refresh::Refresh, QuicAddress};
use std::io;
use tokio::sync::Semaphore;

fn named(
    expected: &ExpectedPeerIdentity,
    initial: SocketAddr,
    slots: Arc<Semaphore>,
    lookup: impl Fn(&str) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
) -> QuicPeer {
    let mut peer = QuicPeer::open(
        expected,
        QuicAddress::from_open("moving.test:9560", vec![initial]),
    )
    .unwrap();
    Arc::get_mut(&mut peer.0).unwrap().refresh =
        Some(Refresh::with_lookup("moving.test:9560", slots, lookup));
    peer
}

#[test]
fn literals_and_explicit_query_snapshots_retain_no_hostname() {
    let initial: SocketAddr = "127.0.0.1:9560".parse().unwrap();
    assert!(QuicAddress::from_open(&initial.to_string(), vec![initial])
        .hostname
        .is_none());
    assert!(QuicAddress::from(vec![initial]).hostname.is_none());
    assert_eq!(
        QuicAddress::from_open("moving.test:9560", vec![initial])
            .hostname
            .as_deref(),
        Some("moving.test:9560")
    );
}

#[tokio::test]
async fn a_resolution_failure_never_uses_the_open_time_answer() {
    let selected = generate_identity();
    let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer = named(
        &expected(&selected),
        silent.local_addr().unwrap(),
        Arc::new(Semaphore::new(1)),
        |_| Err(io::ErrorKind::NotFound.into()),
    );
    let error = timeout(Duration::from_secs(1), peer.dial())
        .await
        .expect("cached silent address was dialled")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AddrNotAvailable);
    assert!(hs::no_connection_made(&error));
    let mut packet = [0u8; 2048];
    assert!(timeout(Duration::from_millis(20), silent.recv(&mut packet))
        .await
        .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolution_and_address_racing_share_the_original_five_second_window() {
    let selected = generate_identity();
    let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = silent.local_addr().unwrap();
    let peer = named(
        &expected(&selected),
        address,
        Arc::new(Semaphore::new(1)),
        move |_| {
            std::thread::sleep(Duration::from_secs(1));
            Ok(vec![address])
        },
    );
    let start = Instant::now();
    assert_eq!(
        peer.dial().await.unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert!(start.elapsed() >= hs::READ_WINDOW);
    assert!(start.elapsed() < hs::READ_WINDOW + Duration::from_millis(500));
}

#[tokio::test]
async fn capacity_wait_is_bounded_and_does_not_delay_literal_dials() {
    let selected = generate_identity();
    let closed = reserve_closed_udp();
    let address = closed.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let hostname = named(
        &expected(&selected),
        address,
        Arc::new(Semaphore::new(0)),
        move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(vec![address])
        },
    );
    let literal = QuicPeer::open(&expected(&selected), vec![address]).unwrap();
    assert!(literal.0.refresh.is_none());
    drop(closed);
    let started = Instant::now();
    let pending = tokio::spawn(async move { hostname.dial().await });
    assert_eq!(
        timeout(Duration::from_secs(1), literal.dial())
            .await
            .unwrap()
            .unwrap_err()
            .kind(),
        io::ErrorKind::ConnectionRefused
    );
    assert_eq!(
        pending.await.unwrap().unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert!(started.elapsed() < hs::READ_WINDOW + Duration::from_millis(500));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refreshed_families_reuse_endpoints_keep_sessions_and_reject_a_wrong_key() {
    timeout(Duration::from_secs(15), async {
        let selected = generate_identity();
        let (v4, _) = server_endpoint(&selected);
        let Ok(v6) = std::panic::catch_unwind(|| {
            server_endpoint_at(&selected, "[::1]:0".parse().unwrap()).0
        }) else {
            eprintln!("skipped: no IPv6 loopback on this host");
            return;
        };
        let (wrong, _) = server_endpoint(&generate_identity());
        let (v4_addr, v6_addr, wrong_addr) = (
            v4.local_addr().unwrap(),
            v6.local_addr().unwrap(),
            wrong.local_addr().unwrap(),
        );
        let answers = StdMutex::new(std::collections::VecDeque::from([
            Ok(vec![v6_addr]),
            Err(io::ErrorKind::NotFound.into()),
            Ok(vec![wrong_addr]),
            Ok(vec![v4_addr]),
        ]));
        let peer = named(
            &expected(&selected),
            v4_addr,
            Arc::new(Semaphore::new(1)),
            move |_| answers.lock().unwrap().pop_front().unwrap(),
        );
        let v4_local = peer
            .0
            .v4
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(peer.0.v6.lock().unwrap().is_none());
        let (server_side, client_side) =
            tokio::join!(async { v6.accept().await.unwrap().await.unwrap() }, async {
                peer.dial().await.unwrap()
            },);
        assert_eq!(client_side.remote_address(), v6_addr);
        let v6_local = peer
            .0
            .v6
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .local_addr()
            .unwrap();
        let (mut send, _recv) = client_side.open_bi().await.unwrap();
        send.write_all(b"before").await.unwrap();
        let (_send, mut recv) = server_side.accept_bi().await.unwrap();
        let mut data = [0u8; 6];
        recv.read_exact(&mut data).await.unwrap();
        assert_eq!(&data, b"before");
        assert!(hs::no_connection_made(&peer.dial().await.unwrap_err()));

        let observer = tokio::spawn(async move {
            match wrong.accept().await.unwrap().await {
                Err(_) => false,
                Ok(connection) => timeout(Duration::from_millis(500), connection.accept_bi())
                    .await
                    .is_ok_and(|result| result.is_ok()),
            }
        });
        let error = connect_v3_on(
            &peer,
            "127.0.0.1:8080",
            &generate_identity(),
            &expected(&selected),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            !observer.await.unwrap(),
            "wrong identity received an application stream"
        );
        let (_, latest) =
            tokio::join!(async { v4.accept().await.unwrap().await.unwrap() }, async {
                peer.dial().await.unwrap()
            },);
        assert_eq!(latest.remote_address(), v4_addr);
        assert_eq!(
            peer.0
                .v4
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .local_addr()
                .unwrap(),
            v4_local
        );
        assert_eq!(
            peer.0
                .v6
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .local_addr()
                .unwrap(),
            v6_local
        );
        send.write_all(b"after!").await.unwrap();
        recv.read_exact(&mut data).await.unwrap();
        assert_eq!(&data, b"after!");
        latest.close(0u32.into(), b"");
        client_side.close(0u32.into(), b"");
    })
    .await
    .unwrap();
}
