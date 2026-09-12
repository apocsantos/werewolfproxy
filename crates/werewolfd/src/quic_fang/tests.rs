use super::*;
use crate::{
    state::DaemonState,
    target_policy::TargetPolicy,
    tls_identity::{self, RuntimeTlsIdentity},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, UdpSocket},
    sync::{oneshot, Mutex},
    time::{sleep, timeout, Instant},
};
use werewolf_core::{
    pack::{PeerRecord, TrustLevel},
    pelt::{generate_identity, PeltIdentity},
};

struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn peer(identity: &PeltIdentity, name: &str) -> PeerRecord {
    PeerRecord {
        name: name.into(),
        fingerprint: identity.fingerprint.clone(),
        public_key_b64: Some(identity.public_key_b64.clone()),
        address: "quic://127.0.0.1:1".into(),
        trust: TrustLevel::Packmate,
    }
}

fn expected(identity: &PeltIdentity) -> ExpectedPeerIdentity {
    ExpectedPeerIdentity {
        fingerprint: identity.fingerprint.clone(),
        public_key_b64: Some(identity.public_key_b64.clone()),
    }
}

fn reserve_udp() -> SocketAddr {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn server_endpoint(identity: &PeltIdentity) -> (Endpoint, Vec<u8>) {
    let identity = RuntimeTlsIdentity::from_pelt(identity).unwrap();
    let certificate = identity.certificate().as_ref().to_vec();
    let crypto = tls_identity::server_config(&identity).unwrap();
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(crypto).unwrap();
    let config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    (
        Endpoint::server(config, "127.0.0.1:0".parse().unwrap()).unwrap(),
        certificate,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_correct_receiver_and_fifty_mib_integrity() {
    timeout(Duration::from_secs(30), async {
        let sender = generate_identity();
        let receiver = generate_identity();
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target.local_addr().unwrap();
        let runtime_tls_identity = Arc::new(RuntimeTlsIdentity::from_pelt(&receiver).unwrap());
        let state = Arc::new(Mutex::new(DaemonState {
            pelt: Some(receiver.clone()),
            runtime_tls_identity: Some(runtime_tls_identity),
            peers: vec![peer(&sender, "sender")],
            target_policy: TargetPolicy::Grants(HashMap::from([(
                sender.fingerprint.clone(),
                [target_address].into(),
            )])),
            ..Default::default()
        }));
        let address = reserve_udp();
        let listener_state = state.clone();
        let listener = AbortOnDrop(tokio::spawn(async move {
            crate::transport::run_quic_fang_listener(&address.to_string(), listener_state).await
        }));
        sleep(Duration::from_millis(30)).await;

        let (connection, mut send, _recv) = connect_v3(
            address,
            &target_address.to_string(),
            &sender,
            &expected(&receiver),
        )
        .await
        .unwrap();
        let (mut target_stream, _) = target.accept().await.unwrap();

        const SIZE: usize = 50 * 1024 * 1024;
        let chunk: Vec<u8> = (0..64 * 1024)
            .map(|index| (index as u8).wrapping_mul(31).wrapping_add(17))
            .collect();
        let mut expected_hash = blake3::Hasher::new();
        let writer = async {
            let mut remaining = SIZE;
            while remaining != 0 {
                let amount = remaining.min(chunk.len());
                expected_hash.update(&chunk[..amount]);
                send.write_all(&chunk[..amount]).await.unwrap();
                remaining -= amount;
            }
            send.finish().unwrap();
            *expected_hash.finalize().as_bytes()
        };
        let reader = async {
            let mut actual_hash = blake3::Hasher::new();
            let mut remaining = SIZE;
            let mut buffer = vec![0; 64 * 1024];
            while remaining != 0 {
                let amount = target_stream.read(&mut buffer).await.unwrap();
                assert_ne!(amount, 0);
                actual_hash.update(&buffer[..amount]);
                remaining -= amount;
            }
            *actual_hash.finalize().as_bytes()
        };
        let (expected_hash, actual_hash) = tokio::join!(writer, reader);
        assert_eq!(actual_hash, expected_hash);
        connection.close(0u32.into(), b"");
        drop(listener);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_post_ack_target_close_preserves_authenticated_ack() {
    timeout(Duration::from_secs(10), async {
        let sender = generate_identity();
        let receiver = generate_identity();
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target.local_addr().unwrap();
        let target_task = AbortOnDrop(tokio::spawn(async move {
            let (stream, _) = target.accept().await.unwrap();
            drop(stream);
        }));
        let runtime_tls_identity = Arc::new(RuntimeTlsIdentity::from_pelt(&receiver).unwrap());
        let state = Arc::new(Mutex::new(DaemonState {
            pelt: Some(receiver.clone()),
            runtime_tls_identity: Some(runtime_tls_identity),
            peers: vec![peer(&sender, "sender")],
            target_policy: TargetPolicy::Grants(HashMap::from([(
                sender.fingerprint.clone(),
                [target_address].into(),
            )])),
            ..Default::default()
        }));
        let address = reserve_udp();
        let listener = AbortOnDrop(tokio::spawn(async move {
            crate::transport::run_quic_fang_listener(&address.to_string(), state).await
        }));
        sleep(Duration::from_millis(30)).await;

        let (connection, _send, _recv) = connect_v3(
            address,
            &target_address.to_string(),
            &sender,
            &expected(&receiver),
        )
        .await
        .unwrap();
        connection.close(0u32.into(), b"");
        drop((target_task, listener));
    })
    .await
    .unwrap();
}

async fn rejected_server_stream_count(
    server_identity: PeltIdentity,
    expected_identity: PeltIdentity,
) -> usize {
    let sender = generate_identity();
    let (server, _) = server_endpoint(&server_identity);
    let address = server.local_addr().unwrap();
    let stream_count = Arc::new(AtomicUsize::new(0));
    let observed = stream_count.clone();
    let mut receiver = AbortOnDrop(tokio::spawn(async move {
        let Ok(Some(incoming)) = timeout(Duration::from_secs(3), server.accept()).await else {
            return;
        };
        let Ok(Ok(connection)) = timeout(Duration::from_secs(3), incoming).await else {
            return;
        };
        if let Ok(Ok((_send, mut recv))) =
            timeout(Duration::from_millis(500), connection.accept_bi()).await
        {
            observed.fetch_add(1, Ordering::SeqCst);
            let _ = recv.read_to_end(hs::MESSAGE_LIMIT).await;
        }
    }));
    assert!(connect_v3(
        address,
        "203.0.113.77:4242",
        &sender,
        &expected(&expected_identity),
    )
    .await
    .is_err());
    timeout(Duration::from_secs(4), &mut receiver.0)
        .await
        .unwrap()
        .unwrap();
    stream_count.load(Ordering::SeqCst)
}

#[tokio::test]
async fn wrong_receiver_rejects_before_application_stream() {
    let expected_identity = generate_identity();
    let wrong_identity = generate_identity();
    assert_eq!(
        rejected_server_stream_count(wrong_identity, expected_identity).await,
        0
    );
}

#[tokio::test]
async fn different_valid_pack_peer_rejects_before_application_stream() {
    let selected = generate_identity();
    let other_pack_peer = generate_identity();
    werewolf_core::state_validation::pack(&[
        peer(&selected, "selected"),
        peer(&other_pack_peer, "other"),
    ])
    .unwrap();
    assert_eq!(
        rejected_server_stream_count(other_pack_peer, selected).await,
        0
    );
}

async fn assert_preconnect_key_failure(public_key_b64: Option<String>) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let sender = generate_identity();
    let expected_peer = ExpectedPeerIdentity {
        fingerprint: generate_identity().fingerprint,
        public_key_b64,
    };
    assert!(
        connect_v3(address, "203.0.113.1:1", &sender, &expected_peer)
            .await
            .is_err()
    );
    let mut datagram = [0; 2048];
    assert!(
        timeout(Duration::from_millis(150), socket.recv_from(&mut datagram))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn missing_pack_key_fails_before_quic_datagram() {
    assert_preconnect_key_failure(None).await;
}

#[tokio::test]
async fn invalid_pack_key_fails_before_quic_datagram() {
    assert_preconnect_key_failure(Some("not-canonical-base64".into())).await;
}

async fn relay(
    socket: UdpSocket,
    server: SocketAddr,
    capture: Arc<StdMutex<Vec<Vec<u8>>>>,
    mut stop: oneshot::Receiver<()>,
) {
    let mut client = None;
    let mut buffer = vec![0; 65_535];
    loop {
        tokio::select! {
            _ = &mut stop => break,
            received = socket.recv_from(&mut buffer) => {
                let Ok((amount, source)) = received else { break };
                capture.lock().unwrap().push(buffer[..amount].to_vec());
                if source == server {
                    if let Some(client) = client {
                        let _ = socket.send_to(&buffer[..amount], client).await;
                    }
                } else {
                    client = Some(source);
                    let _ = socket.send_to(&buffer[..amount], server).await;
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_client_has_no_sni_or_alpn_and_preserves_exporter_transcripts() {
    timeout(Duration::from_secs(10), async {
        let sender = generate_identity();
        let receiver = generate_identity();
        let remote = "198.51.100.42:65000";
        let (server, certificate) = server_endpoint(&receiver);
        let server_address = server.local_addr().unwrap();
        let relay_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let relay_address = relay_socket.local_addr().unwrap();
        let capture = Arc::new(StdMutex::new(Vec::new()));
        let (stop_tx, stop_rx) = oneshot::channel();
        let mut relay_task = AbortOnDrop(tokio::spawn(relay(
            relay_socket,
            server_address,
            capture.clone(),
            stop_rx,
        )));
        let receiver_for_server = receiver.clone();
        let sender_for_server = sender.clone();
        let mut server_task = AbortOnDrop(tokio::spawn(async move {
            let connection = server.accept().await.unwrap().await.unwrap();
            let handshake = connection
                .handshake_data()
                .unwrap()
                .downcast::<quinn::crypto::rustls::HandshakeData>()
                .unwrap();
            assert_eq!(handshake.server_name, None);
            assert_eq!(handshake.protocol, None);
            let binding = hs::quic_binding(&connection).unwrap();
            let (mut send, mut recv) = connection.accept_bi().await.unwrap();
            let open: hs::QuicOpen = hs::read(
                &mut recv,
                hs::MESSAGE_LIMIT,
                Instant::now() + hs::READ_WINDOW,
            )
            .await
            .unwrap();
            assert_eq!(open.cmd, "fang.quic.open");
            assert_eq!(open.protocol, hs::QUIC);
            assert_eq!(open.sender_fingerprint, sender_for_server.fingerprint);
            assert_eq!(open.receiver_fingerprint, receiver_for_server.fingerprint);
            assert_eq!(open.remote, remote);
            let retained = open.transcript(&binding).unwrap();
            hs::verify(
                &sender_for_server.public_key_b64,
                &retained,
                &open.signature,
            )
            .unwrap();
            let ack = hs::QuicAck::new(&open, &retained, &receiver_for_server).unwrap();
            assert_eq!(ack.cmd, "fang.quic.ack");
            assert_eq!(ack.protocol, hs::QUIC);
            hs::write(
                &mut send,
                &ack,
                hs::MESSAGE_LIMIT,
                Instant::now() + hs::READ_WINDOW,
            )
            .await
            .unwrap();
            let application = recv.read_to_end(512).await.unwrap();
            (binding, open, application)
        }));

        let (connection, mut send, _recv) =
            connect_v3(relay_address, remote, &sender, &expected(&receiver))
                .await
                .unwrap();
        let client_binding = hs::quic_binding(&connection).unwrap();
        let peer_identity = connection
            .peer_identity()
            .unwrap()
            .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
            .unwrap();
        assert_eq!(peer_identity[0].as_ref(), certificate);
        let application_sentinel = hs::random::<100>().unwrap();
        send.write_all(&application_sentinel).await.unwrap();
        send.finish().unwrap();
        let (server_binding, open, application) = (&mut server_task.0).await.unwrap();
        assert_eq!(client_binding, server_binding);
        assert_eq!(application, application_sentinel);
        assert_eq!(
            open.transcript(&server_binding).unwrap(),
            open.transcript(&client_binding).unwrap()
        );
        connection.close(0u32.into(), b"");
        sleep(Duration::from_millis(30)).await;
        let _ = stop_tx.send(());
        (&mut relay_task.0).await.unwrap();

        let datagrams = capture.lock().unwrap();
        let raw: Vec<u8> = datagrams.iter().flatten().copied().collect();
        let lower: Vec<u8> = raw.iter().map(u8::to_ascii_lowercase).collect();
        let semantics = [
            "werewolf", "fang", "pelt", "pack", "wwp1", "target", "sender", "receiver", "protocol",
        ];
        for semantic in semantics {
            assert!(!lower
                .windows(semantic.len())
                .any(|part| part == semantic.as_bytes()));
        }
        let raw_scan: Vec<_> = [
            "werewolf", "fang", "pelt", "pack", "wwp1", "target", "sender", "receiver", "ack",
            "protocol",
        ]
        .into_iter()
        .map(|semantic| {
            (
                semantic,
                lower
                    .windows(semantic.len())
                    .any(|part| part == semantic.as_bytes()),
            )
        })
        .collect();
        eprintln!("STAGE12D2_RAW_DATAGRAM_SCAN {raw_scan:?}");
        for exact in [
            sender.fingerprint.as_bytes(),
            receiver.fingerprint.as_bytes(),
            remote.as_bytes(),
            sender.public_key_b64.as_bytes(),
            receiver.public_key_b64.as_bytes(),
            certificate.as_slice(),
            application_sentinel.as_slice(),
        ] {
            assert!(!raw.windows(exact.len()).any(|part| part == exact));
        }
        let receiver_raw = STANDARD.decode(&receiver.public_key_b64).unwrap();
        assert!(!raw
            .windows(receiver_raw.len())
            .any(|part| part == receiver_raw));
        let receiver_spki =
            tls_identity::canonical_spki_from_public_key_b64(&receiver.public_key_b64).unwrap();
        assert!(!raw
            .windows(receiver_spki.len())
            .any(|part| part == receiver_spki));
    })
    .await
    .unwrap();
}
