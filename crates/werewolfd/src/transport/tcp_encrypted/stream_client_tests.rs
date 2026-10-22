//! Stage26 step 1: the encrypted-TCP client runs over any byte stream, not
//! only a socket it opened itself. An unmodified receiver must accept a
//! session carried through an in-memory pipe exactly as it accepts a direct
//! one, and the selected-peer TLS pin must still hold.
use super::authority_tests::fixture;
use super::*;
use crate::target_policy::TargetPolicy;
use tokio::time::timeout;
use werewolf_core::pelt::generate_identity;

struct Receiver {
    address: std::net::SocketAddr,
    target: std::net::SocketAddr,
    expected: crate::policy::ExpectedPeerIdentity,
    _worker: super::authority_tests::Task,
    _echo: super::authority_tests::Task,
}

/// One receiver connection with the real server path, and an echo target
/// granted to `sender`.
async fn receiver(sender: &PeltIdentity) -> Receiver {
    let state = fixture(&[sender]);
    let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = target_listener.local_addr().unwrap();
    {
        let mut state = state.lock().await;
        let TargetPolicy::Grants(grants) = &mut state.target_policy else {
            panic!("fixture grants");
        };
        grants.insert(sender.fingerprint.clone(), [target].into());
    }
    let echo = super::authority_tests::Task(tokio::spawn(async move {
        let (mut stream, _) = target_listener.accept().await?;
        let (mut r, mut w) = stream.split();
        tokio::io::copy(&mut r, &mut w).await?;
        w.shutdown().await
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let pelt = state.lock().await.pelt.clone().unwrap();
    let expected = crate::policy::ExpectedPeerIdentity {
        fingerprint: pelt.fingerprint.clone(),
        public_key_b64: Some(pelt.public_key_b64.clone()),
    };
    let identity = state.lock().await.runtime_tls_identity.clone().unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(
        crate::tls_identity::server_config(&identity).unwrap(),
    ));
    let permit = state.lock().await.admission.handshake().unwrap();
    let worker = super::authority_tests::Task(tokio::spawn(async move {
        let (accepted, _) = listener.accept().await?;
        let started = tokio::time::Instant::now();
        let tls = acceptor.accept(AuthorityTcpStream::new(accepted)).await?;
        handle_fang_pipe(tls, state, permit, started).await
    }));
    Receiver {
        address,
        target,
        expected,
        _worker: worker,
        _echo: echo,
    }
}

/// A byte stream to `address` that is not a socket: an in-memory duplex
/// whose far end is copied to and from a TCP connection, as a relay would.
async fn piped(address: std::net::SocketAddr) -> tokio::io::DuplexStream {
    let (near, mut far) = tokio::io::duplex(64 * 1024);
    let mut tcp = TcpStream::connect(address).await.unwrap();
    tokio::spawn(async move {
        let _ = tokio::io::copy_bidirectional(&mut far, &mut tcp).await;
    });
    near
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_through_a_non_socket_stream_is_accepted_and_exact() {
    const BYTES: usize = 1 << 20;
    timeout(Duration::from_secs(30), async {
        let sender = generate_identity();
        let receiver = receiver(&sender).await;
        let stream = piped(receiver.address).await;
        let mut tls = connect_authenticated_over(stream, receiver.address.ip(), &receiver.expected)
            .await
            .unwrap();
        let key = client_handshake(
            &mut tls,
            &receiver.target.to_string(),
            &sender,
            &receiver.expected,
        )
        .await
        .unwrap();

        // The application side of the Fang, as `pipe_one_fang_connection` uses it.
        let local = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut app = TcpStream::connect(local.local_addr().unwrap())
            .await
            .unwrap();
        let (mut inbound, _) = local.accept().await.unwrap();
        let copier =
            tokio::spawn(async move { secure_copy_client_side(&mut inbound, tls, key).await });

        let data: Vec<u8> = (0..BYTES).map(|i| (i * 31 % 251) as u8).collect();
        let (mut app_r, mut app_w) = app.split();
        let send = async {
            app_w.write_all(&data).await?;
            app_w.shutdown().await
        };
        let mut echoed = Vec::with_capacity(BYTES);
        let receive = app_r.read_to_end(&mut echoed);
        let (sent, received) = tokio::join!(send, receive);
        sent.unwrap();
        assert_eq!(received.unwrap(), BYTES);
        assert!(echoed == data, "echo through a piped session differs");
        copier.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn the_selected_peer_pin_holds_over_a_non_socket_stream() {
    timeout(Duration::from_secs(10), async {
        let sender = generate_identity();
        let receiver = receiver(&sender).await;
        let impostor = generate_identity();
        let wrong = crate::policy::ExpectedPeerIdentity {
            fingerprint: impostor.fingerprint.clone(),
            public_key_b64: Some(impostor.public_key_b64.clone()),
        };
        let stream = piped(receiver.address).await;
        let error = connect_authenticated_over(stream, receiver.address.ip(), &wrong)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

        // A Pack entry without a full key fails before any byte is sent.
        let fingerprint_only = crate::policy::ExpectedPeerIdentity {
            fingerprint: receiver.expected.fingerprint.clone(),
            public_key_b64: None,
        };
        let (near, mut far) = tokio::io::duplex(1024);
        let error = connect_authenticated_over(near, receiver.address.ip(), &fingerprint_only)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let mut sent = Vec::new();
        far.read_to_end(&mut sent).await.unwrap();
        assert!(sent.is_empty(), "bytes were sent without a pinned key");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_challenge_from_the_wrong_receiver_is_rejected_over_a_stream() {
    timeout(Duration::from_secs(10), async {
        let sender = generate_identity();
        let receiver = receiver(&sender).await;
        let stream = piped(receiver.address).await;
        let mut tls = connect_authenticated_over(stream, receiver.address.ip(), &receiver.expected)
            .await
            .unwrap();
        // TLS was pinned to the right key, but the handshake expects another
        // fingerprint in the challenge: it must stop before sending an OPEN.
        let other = crate::policy::ExpectedPeerIdentity {
            fingerprint: generate_identity().fingerprint.clone(),
            public_key_b64: receiver.expected.public_key_b64.clone(),
        };
        let error = client_handshake(&mut tls, &receiver.target.to_string(), &sender, &other)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    })
    .await
    .unwrap();
}
