use super::*;
use crate::target_policy::TargetPolicy;
use std::time::Duration;
use werewolf_core::{
    nodeid::NodeId,
    pack::{PeerRecord, TrustLevel},
    pelt::generate_identity,
};

fn record(identity: &PeltIdentity, expires_in: u64) -> ContactRecord {
    let now = unix_now();
    ContactRecord::sign(
        identity,
        vec!["quic://203.0.113.7:9560".into()],
        1,
        now + expires_in,
        now,
    )
    .unwrap()
}

/// A record whose validity ended ten seconds ago.
fn expired(identity: &PeltIdentity) -> ContactRecord {
    let now = unix_now();
    ContactRecord::sign(
        identity,
        vec!["quic://203.0.113.7:9560".into()],
        1,
        now - 10,
        now - 100,
    )
    .unwrap()
}

fn find(target: NodeId, k: u8) -> RpcRequest {
    RpcRequest::FindNode { target, k }
}

#[test]
fn limiter_allows_a_burst_then_refills_per_peer() {
    let limiter = RpcLimiter::default();
    let t0 = StdInstant::now();

    for _ in 0..16 {
        assert!(limiter.allow_at("wwp1:A", t0));
    }
    assert!(!limiter.allow_at("wwp1:A", t0));
    assert!(limiter.allow_at("wwp1:B", t0));
    assert!(limiter.allow_at("wwp1:A", t0 + Duration::from_millis(250)));
    assert!(!limiter.allow_at("wwp1:A", t0 + Duration::from_millis(250)));
}

#[test]
fn find_node_orders_by_distance_excludes_requester_and_caps_k() {
    let identities: Vec<_> = (0..6).map(|_| generate_identity()).collect();
    let known: Vec<_> = identities.iter().map(|id| record(id, 3600)).collect();
    let requester = known[0].public_key;
    let target = known[3].node_id();
    let contacts = Contacts {
        own: None,
        known: [known.clone(), vec![expired(&generate_identity())]].concat(),
    };

    let answer = contacts.answer(&find(target.clone(), 3), &requester, unix_now());

    assert_eq!(answer.len(), 3);
    assert_eq!(answer[0], known[3]);
    assert!(answer.iter().all(|r| r.public_key != requester));
    let distances: Vec<_> = answer
        .iter()
        .map(|r| target.xor_distance(&r.node_id()))
        .collect();
    assert!(distances.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn ping_answers_with_own_record_only_while_valid() {
    let identity = generate_identity();
    let contacts = Contacts {
        own: Some(record(&identity, 3600)),
        known: vec![record(&generate_identity(), 3600)],
    };
    let now = unix_now();

    assert_eq!(contacts.answer(&RpcRequest::Ping, &[0; 32], now).len(), 1);
    assert!(contacts
        .answer(&RpcRequest::Ping, &[0; 32], now + 3600)
        .is_empty());
}

#[test]
fn fit_drops_farthest_records_until_the_line_fits() {
    let records: Vec<_> = (0..20)
        .map(|_| record(&generate_identity(), 3600))
        .collect();
    let kept = fit(records.clone(), |records| {
        Ok("x".repeat(records.len() * hs::RESULT_LIMIT / 8))
    })
    .unwrap();

    assert!(kept.len() < hs::RESULT_LIMIT);
    assert!(fit(Vec::new(), |_| Ok("x".repeat(hs::RESULT_LIMIT))).is_err());
}

struct Node {
    receiver: PeltIdentity,
    state: Arc<Mutex<DaemonState>>,
    tcp: SocketAddr,
    quic: SocketAddr,
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Node {
    fn drop(&mut self) {
        for task in &self._tasks {
            task.abort();
        }
    }
}

fn peer(identity: &PeltIdentity) -> PeerRecord {
    PeerRecord {
        name: identity.fingerprint.clone(),
        fingerprint: identity.fingerprint.clone(),
        public_key_b64: Some(identity.public_key_b64.clone()),
        address: "quic://127.0.0.1:1".into(),
        trust: TrustLevel::Packmate,
    }
}

fn free(udp: bool) -> SocketAddr {
    if udp {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    } else {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    }
}

async fn node(packmates: &[&PeltIdentity], contacts: Contacts) -> Node {
    let receiver = generate_identity();
    let tls = Arc::new(crate::tls_identity::RuntimeTlsIdentity::from_pelt(&receiver).unwrap());
    let state = Arc::new(Mutex::new(DaemonState {
        pelt: Some(receiver.clone()),
        runtime_tls_identity: Some(tls),
        peers: packmates.iter().map(|id| peer(id)).collect(),
        target_policy: TargetPolicy::Grants(HashMap::new()),
        contacts,
        ..Default::default()
    }));
    let (tcp, quic) = (free(false), free(true));
    let (tcp_ready, tcp_rx) = tokio::sync::oneshot::channel();
    let (quic_ready, quic_rx) = tokio::sync::oneshot::channel();
    let (a, b) = (state.clone(), state.clone());
    let tasks = vec![
        tokio::spawn(async move {
            let _ = crate::transport::run_fang_listener_with_ready(&tcp.to_string(), a, tcp_ready)
                .await;
        }),
        tokio::spawn(async move {
            let _ = crate::transport::run_quic_fang_listener_with_ready(
                &quic.to_string(),
                b,
                quic_ready,
            )
            .await;
        }),
    ];
    tcp_rx.await.unwrap().unwrap();
    quic_rx.await.unwrap().unwrap();
    Node {
        receiver,
        state,
        tcp,
        quic,
        _tasks: tasks,
    }
}

fn expected(node: &Node) -> ExpectedPeerIdentity {
    ExpectedPeerIdentity {
        fingerprint: node.receiver.fingerprint.clone(),
        public_key_b64: Some(node.receiver.public_key_b64.clone()),
    }
}

async fn both(
    node: &Node,
    sender: &PeltIdentity,
    request: &RpcRequest,
) -> (
    io::Result<Vec<ContactRecord>>,
    io::Result<Vec<ContactRecord>>,
) {
    let expected = expected(node);
    (
        query_quic(node.quic, sender, &expected, request).await,
        query_tcp(&node.tcp.to_string(), sender, &expected, request).await,
    )
}

#[tokio::test]
async fn packmate_pings_and_finds_nodes_over_quic_and_tcp() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let sender = generate_identity();
        let known: Vec<_> = (0..4).map(|_| record(&generate_identity(), 3600)).collect();
        let node = node(&[&sender], Contacts::default()).await;
        let own = record(&node.receiver, 3600);
        node.state.lock().await.contacts = Contacts {
            own: Some(own.clone()),
            known: known.clone(),
        };

        let (quic, tcp) = both(&node, &sender, &RpcRequest::Ping).await;
        assert_eq!(quic.unwrap(), vec![own.clone()]);
        assert_eq!(tcp.unwrap(), vec![own]);

        let target = known[2].node_id();
        let (quic, tcp) = both(&node, &sender, &find(target, 2)).await;
        let (quic, tcp) = (quic.unwrap(), tcp.unwrap());
        assert_eq!(quic.len(), 2);
        assert_eq!(quic[0], known[2]);
        assert_eq!(quic, tcp);

        // RPCs never take an inbound authority lease or reservation.
        let summary = node.state.lock().await.inbound_authority.summary();
        assert_eq!(summary["active"], 0);
        assert_eq!(summary["establishing"], 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn non_packmates_silver_and_rate_limit_are_rejected() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (sender, stranger) = (generate_identity(), generate_identity());
        let node = node(&[&sender], Contacts::default()).await;

        let (quic, tcp) = both(&node, &stranger, &RpcRequest::Ping).await;
        assert!(quic.is_err() && tcp.is_err());

        let limiter = node.state.lock().await.rpc_limiter.clone();
        let expected = expected(&node);
        while limiter.allow(&sender.fingerprint) {}
        assert!(query_quic(node.quic, &sender, &expected, &RpcRequest::Ping)
            .await
            .is_err());
        while limiter.allow(&sender.fingerprint) {}
        assert!(
            query_tcp(&node.tcp.to_string(), &sender, &expected, &RpcRequest::Ping)
                .await
                .is_err()
        );

        let other = generate_identity();
        node.state.lock().await.peers.push(peer(&other));
        let (quic, tcp) = both(&node, &other, &RpcRequest::Ping).await;
        assert!(quic.is_ok() && tcp.is_ok());

        node.state.lock().await.inbound_authority.lock().unwrap();
        let (quic, tcp) = both(&node, &other, &RpcRequest::Ping).await;
        assert!(quic.is_err() && tcp.is_err());
    })
    .await
    .unwrap();
}
