use super::*;
use crate::{authority::Authority, target_policy::TargetPolicy};
use std::collections::HashMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use werewolf_core::{
    pack::{PeerRecord, TrustLevel},
    pelt::{generate_identity, PeltIdentity},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcp_and_quic_listeners_waiting_before_pelt_publish_both_activate() {
    timeout(Duration::from_secs(12), async {
        let (observed, mut waits) = tokio::sync::mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(DaemonState {
            tls_identity_wait_observed: Some(observed),
            ..Default::default()
        }));
        let tcp_address = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let quic_address = std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let tcp_state = state.clone();
        let tcp = Task(tokio::spawn(async move {
            crate::transport::run_fang_listener(&tcp_address.to_string(), tcp_state).await
        }));
        let quic_state = state.clone();
        let quic = Task(tokio::spawn(async move {
            run_quic_fang_listener(&quic_address.to_string(), quic_state).await
        }));

        // Each signal is emitted after an actual production listener checked
        // state and found no identity. No scheduling sleep establishes order.
        for _ in 0..2 {
            timeout(Duration::from_secs(2), waits.recv())
                .await
                .unwrap()
                .unwrap();
        }
        let pelt = generate_identity();
        let identity = Arc::new(crate::tls_identity::RuntimeTlsIdentity::from_pelt(&pelt).unwrap());
        {
            let mut live = state.lock().await;
            assert!(!live.tls_identity_ready.is_ready());
            live.pelt = Some(pelt.clone());
            live.runtime_tls_identity = Some(identity);
            live.tls_identity_ready.publish();
        }

        let expected = crate::policy::ExpectedPeerIdentity {
            fingerprint: pelt.fingerprint.clone(),
            public_key_b64: Some(pelt.public_key_b64.clone()),
        };
        let tcp_client = timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(stream) = crate::transport::tcp_encrypted::connect_authenticated_tcp(
                    &tcp_address.to_string(),
                    &expected,
                )
                .await
                {
                    break stream;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            tcp_client.get_ref().1.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_3)
        );

        let rustls =
            crate::tls_identity::client_config_for_selected_peer_key(Some(&pelt.public_key_b64))
                .unwrap();
        let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(rustls).unwrap();
        let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic_crypto)));
        let connection = timeout(
            Duration::from_secs(5),
            endpoint
                .connect(quic_address, &quic_address.ip().to_string())
                .unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(connection.remote_address(), quic_address);
        connection.close(hs::V3_REJECT_CODE.into(), b"");
        drop(tcp_client);
        drop(tcp);
        drop(quic);
    })
    .await
    .unwrap();
}

struct Task(tokio::task::JoinHandle<io::Result<()>>);
impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct Stream {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    target: TcpStream,
}

async fn open(
    connection: &quinn::Connection,
    state: &Arc<Mutex<DaemonState>>,
    sender: &PeltIdentity,
) -> Stream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = {
        let mut state = state.lock().await;
        let TargetPolicy::Grants(grants) = &mut state.target_policy else {
            panic!("fixture grants");
        };
        grants.insert(sender.fingerprint.clone(), [address].into());
        state.pelt.as_ref().unwrap().fingerprint.clone()
    };
    let binding = hs::quic_binding(connection).unwrap();
    let request = hs::QuicOpen::new(sender, &receiver, &address.to_string(), &binding).unwrap();
    let retained = request.transcript(&binding).unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    hs::write(&mut send, &request, hs::MESSAGE_LIMIT, deadline)
        .await
        .unwrap();
    let ack: hs::QuicAck = hs::read(&mut recv, hs::MESSAGE_LIMIT, deadline)
        .await
        .unwrap();
    hs::verify(
        &ack.receiver_pubkey,
        &ack.transcript(&request, &retained).unwrap(),
        &ack.signature,
    )
    .unwrap();
    let (target, _) = listener.accept().await.unwrap();
    Stream { send, recv, target }
}

#[tokio::test]
async fn multiplexed_peer_revoke_preserves_other_stream_and_silver_closes_connection() {
    timeout(Duration::from_secs(15), async {
        let a = generate_identity();
        let b = generate_identity();
        let receiver = generate_identity();
        let authority = Authority::new(false);
        let runtime_tls_identity =
            Arc::new(crate::tls_identity::RuntimeTlsIdentity::from_pelt(&receiver).unwrap());
        let state = Arc::new(Mutex::new(DaemonState {
            inbound_authority: authority.clone(),
            pelt: Some(receiver),
            runtime_tls_identity: Some(runtime_tls_identity),
            peers: [&a, &b]
                .into_iter()
                .map(|p| PeerRecord {
                    name: p.fingerprint.clone(),
                    fingerprint: p.fingerprint.clone(),
                    public_key_b64: Some(p.public_key_b64.clone()),
                    address: "quic://127.0.0.1:1".into(),
                    trust: TrustLevel::Packmate,
                })
                .collect(),
            target_policy: TargetPolicy::Grants(HashMap::new()),
            ..Default::default()
        }));
        let address = std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let copy = state.clone();
        let _server = Task(tokio::spawn(async move {
            run_quic_fang_listener(&address.to_string(), copy).await
        }));
        let client = crate::quic_lab::make_client_endpoint().unwrap();
        let connection = client.connect(address, "localhost").unwrap().await.unwrap();
        let mut first = open(&connection, &state, &a).await;
        let mut second = open(&connection, &state, &b).await;
        first.send.write_all(b"before").await.unwrap();
        let mut before = [0u8; 6];
        first.target.read_exact(&mut before).await.unwrap();
        assert_eq!(&before, b"before");
        authority.deny_peer(&a.fingerprint).unwrap();
        authority.cleanup(Some(&a.fingerprint)).await.unwrap();
        assert!(first.recv.read_to_end(64).await.is_err());
        let mut rest = Vec::new();
        first.target.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
        // A QUIC connection is not a peer identity. B retains its own lease.
        second.send.write_all(b"other").await.unwrap();
        let mut other = [0u8; 5];
        second.target.read_exact(&mut other).await.unwrap();
        assert_eq!(&other, b"other");
        second.target.write_all(b"reply").await.unwrap();
        second.recv.read_exact(&mut other).await.unwrap();
        assert_eq!(&other, b"reply");
        authority.lock().unwrap();
        authority.cleanup(None).await.unwrap();
        connection.closed().await;
        assert!(second.recv.read_to_end(64).await.is_err());
        // Both registry leases are gone; Stage 10 connection-owned replay
        // reservations were not touched by the individual peer cancellation.
    })
    .await
    .unwrap();
}
