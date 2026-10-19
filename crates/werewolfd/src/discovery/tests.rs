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
    let stranger = record(&generate_identity(), 3600);
    let mut pack: Vec<_> = known.iter().map(|r| r.public_key).collect();
    let lapsed = expired(&generate_identity());
    pack.push(lapsed.public_key);
    let contacts = Contacts {
        known: [known.clone(), vec![lapsed, stranger.clone()]].concat(),
        ..Default::default()
    };

    let answer = contacts.answer(&find(target.clone(), 3), &requester, &pack, unix_now());

    assert_eq!(answer.len(), 3);
    assert_eq!(answer[0], known[3]);
    assert!(answer.iter().all(|r| r.public_key != requester));
    let distances: Vec<_> = answer
        .iter()
        .map(|r| target.xor_distance(&r.node_id()))
        .collect();
    assert!(distances.windows(2).all(|pair| pair[0] <= pair[1]));

    // Records of nodes outside the responder's Pack are never relayed.
    let everything = contacts.answer(&find(stranger.node_id(), 20), &requester, &pack, unix_now());
    assert_eq!(everything.len(), known.len() - 1);
    assert!(!everything.contains(&stranger));
}

#[test]
fn ping_answers_with_own_record_only_while_valid() {
    let identity = generate_identity();
    let contacts = Contacts {
        own: Some(record(&identity, 3600)),
        known: vec![record(&generate_identity(), 3600)],
        ..Default::default()
    };
    let now = unix_now();

    assert_eq!(
        contacts.answer(&RpcRequest::Ping, &[0; 32], &[], now).len(),
        1
    );
    assert!(contacts
        .answer(&RpcRequest::Ping, &[0; 32], &[], now + 3600)
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
        let members: Vec<_> = (0..4).map(|_| generate_identity()).collect();
        let known: Vec<_> = members.iter().map(|id| record(id, 3600)).collect();
        let pack: Vec<_> = [&sender].into_iter().chain(&members).collect();
        let node = node(&pack, Contacts::default()).await;
        let advertised = vec!["tcp://198.51.100.4:9560".to_string()];
        node.state.lock().await.contacts = Contacts {
            advertised: advertised.clone(),
            known: known.clone(),
            ..Default::default()
        };

        // The responder signs its own record from the advertised addresses.
        let (quic, tcp) = both(&node, &sender, &RpcRequest::Ping).await;
        let (quic, tcp) = (quic.unwrap(), tcp.unwrap());
        assert_eq!(quic, tcp);
        assert_eq!(quic.len(), 1);
        assert_eq!(quic[0].addresses, advertised);
        assert_eq!(
            quic[0].public_key,
            hs::binary::<32>(&node.receiver.public_key_b64).unwrap()
        );

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

fn member(name: &str, node: &Node) -> PeerRecord {
    PeerRecord {
        name: name.into(),
        address: format!("quic://{}", node.quic),
        ..peer(&node.receiver)
    }
}

fn temporary_den() -> (
    std::path::PathBuf,
    Arc<werewolf_core::local_fs::PrivateDirectory>,
) {
    let path = std::env::temp_dir().join(format!(
        "wwp-discovery-{}",
        hs::hex(&hs::random::<16>().unwrap())
    ));
    let den = werewolf_core::local_fs::PrivateDirectory::open(&path, true).unwrap();
    (path, Arc::new(den))
}

async fn operate(node: &Node, cmd: &str, args: serde_json::Value) -> serde_json::Value {
    let request = werewolf_core::protocol::ControlRequest {
        id: "test".into(),
        cmd: cmd.into(),
        args,
    };
    let response = operator::handle(request, node.state.clone()).await;
    serde_json::to_value(response).unwrap()
}

#[tokio::test]
async fn find_returns_a_friend_of_a_friend_without_trusting_it() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (a, b, c) = (
            node(&[], Contacts::default()).await,
            node(&[], Contacts::default()).await,
            node(&[], Contacts::default()).await,
        );
        let mut dens = Vec::new();
        for (node, pack) in [
            (&a, vec![member("wolf-b", &b)]),
            (&b, vec![member("wolf-a", &a), member("wolf-c", &c)]),
            (&c, vec![member("wolf-b", &b)]),
        ] {
            let (path, den) = temporary_den();
            dens.push(path);
            let mut st = node.state.lock().await;
            st.peers = pack;
            st.den = Some(den);
        }
        let advertised = format!("quic://{}", c.quic);
        c.state.lock().await.contacts.advertised = vec![advertised.clone()];

        // B learns C's own record directly from C.
        let refreshed = operate(&b, "discover.refresh", serde_json::json!({})).await;
        assert_eq!(refreshed["ok"], true, "{refreshed}");
        let statuses = refreshed["result"]["queried"].as_array().unwrap();
        assert!(statuses
            .iter()
            .any(|s| s["peer"] == "wolf-c" && s["status"] == "ok" && s["records"] == 1));
        assert_eq!(refreshed["result"]["cached"], true);
        assert_eq!(b.state.lock().await.contacts.known.len(), 1);

        // A finds C through B, which vouches only for its own Pack members.
        let found = operate(
            &a,
            "discover.find",
            serde_json::json!({"target": c.receiver.public_key_b64}),
        )
        .await;
        assert_eq!(found["ok"], true, "{found}");
        let record = &found["result"]["found"];
        assert_eq!(record["public_key_b64"], c.receiver.public_key_b64.as_str());
        assert_eq!(record["addresses"][0], advertised.as_str());
        assert!(record["pack_name"].is_null());

        // Discovery never changes trust: C is cached but not in A's Pack.
        {
            let st = a.state.lock().await;
            assert_eq!(st.peers.len(), 1);
            assert_eq!(st.contacts.known.len(), 1);
        }
        let contacts = operate(&a, "discover.contacts", serde_json::json!({})).await;
        assert_eq!(
            contacts["result"][0]["public_key_b64"],
            c.receiver.public_key_b64.as_str()
        );

        // B does not relay C to A once C leaves B's Pack.
        b.state.lock().await.peers.retain(|p| p.name != "wolf-c");
        a.state.lock().await.contacts.known.clear();
        let missing = operate(
            &a,
            "discover.find",
            serde_json::json!({"target": c.receiver.public_key_b64}),
        )
        .await;
        assert!(missing["result"]["found"].is_null(), "{missing}");

        a.state.lock().await.status.mode = werewolf_core::state::WolfMode::Silver;
        let fenced = operate(
            &a,
            "discover.find",
            serde_json::json!({"target": c.receiver.public_key_b64}),
        )
        .await;
        assert_eq!(fenced["error"]["code"], "SILVER_ACTIVE");

        for path in dens {
            let _ = std::fs::remove_dir_all(path);
        }
    })
    .await
    .unwrap();
}
