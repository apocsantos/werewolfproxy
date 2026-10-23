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
fn failed_dials_are_unreachable_and_failed_handshakes_rejected() {
    use io::ErrorKind::*;
    for kind in [
        ConnectionRefused,
        HostUnreachable,
        NetworkUnreachable,
        AddrNotAvailable,
        TimedOut,
    ] {
        assert_eq!(
            operator::failure_status(kind.into()),
            "unreachable",
            "{kind:?}"
        );
    }
    for kind in [
        PermissionDenied,
        InvalidData,
        UnexpectedEof,
        ConnectionReset,
    ] {
        assert_eq!(
            operator::failure_status(kind.into()),
            "rejected",
            "{kind:?}"
        );
    }
    assert_eq!(operator::failure_status(hs::rejected()), "rejected");
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

#[tokio::test]
async fn find_walks_hops_only_through_nodes_that_trust_the_requester() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (a, b, c, d) = (
            node(&[], Contacts::default()).await,
            node(&[], Contacts::default()).await,
            node(&[], Contacts::default()).await,
            node(&[], Contacts::default()).await,
        );
        let mut dens = Vec::new();
        for (node, pack) in [
            (&a, vec![member("wolf-b", &b)]),
            (&b, vec![member("wolf-a", &a), member("wolf-c", &c)]),
            (&c, vec![member("wolf-b", &b), member("wolf-d", &d)]),
            (&d, vec![member("wolf-c", &c)]),
        ] {
            let (path, den) = temporary_den();
            dens.push(path);
            let mut st = node.state.lock().await;
            st.peers = pack;
            st.den = Some(den);
        }
        for node in [&c, &d] {
            node.state.lock().await.contacts.advertised = vec![format!("quic://{}", node.quic)];
        }
        for node in [&b, &c] {
            let refreshed = operate(node, "discover.refresh", serde_json::json!({})).await;
            assert_eq!(refreshed["ok"], true, "{refreshed}");
        }
        let find_d = || {
            operate(
                &a,
                "discover.find",
                serde_json::json!({"target": d.receiver.public_key_b64}),
            )
        };
        let hop = |result: &serde_json::Value, n: u64| -> Vec<serde_json::Value> {
            result["result"]["queried"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| s["hop"] == n)
                .cloned()
                .collect()
        };

        // B points A at C, but C does not trust A and refuses to answer.
        let refused = find_d().await;
        assert!(refused["result"]["found"].is_null(), "{refused}");
        assert_eq!(hop(&refused, 1)[0]["peer"], "wolf-b");
        assert_eq!(hop(&refused, 1)[0]["records"], 1);
        assert_eq!(hop(&refused, 2).len(), 1);
        assert_eq!(hop(&refused, 2)[0]["status"], "rejected");

        // Once C trusts A, the second hop answers with D.
        c.state.lock().await.peers.push(member("wolf-a", &a));
        let found = find_d().await;
        let record = &found["result"]["found"];
        assert_eq!(
            record["public_key_b64"],
            d.receiver.public_key_b64.as_str(),
            "{found}"
        );
        let second = hop(&found, 2);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0]["status"], "ok");
        assert!(second[0]["peer"].as_str().unwrap().starts_with("node:"));
        assert!(hop(&found, 3).is_empty());

        // Neither C nor D became trusted by A.
        assert_eq!(a.state.lock().await.peers.len(), 1);

        for path in dens {
            let _ = std::fs::remove_dir_all(path);
        }
    })
    .await
    .unwrap();
}

// Stage25 `services` op (docs/STAGE25_SERVICES_RPC_WIRE_SPEC.md).

fn published() -> Vec<PublishedService> {
    use werewolf_core::service::ServiceKind;
    [
        ("web", ServiceKind::Http, "127.0.0.1:8080"),
        ("ssh", ServiceKind::Ssh, "127.0.0.1:22"),
        ("db", ServiceKind::Tcp, "10.0.0.5:5432"),
    ]
    .into_iter()
    .map(|(name, kind, target)| PublishedService {
        name: name.into(),
        kind,
        target: target.into(),
    })
    .collect()
}

fn grants(entries: &[(&PeltIdentity, &[&str])]) -> TargetPolicy {
    TargetPolicy::Grants(
        entries
            .iter()
            .map(|(id, targets)| {
                (
                    id.fingerprint.clone(),
                    targets.iter().map(|t| t.parse().unwrap()).collect(),
                )
            })
            .collect(),
    )
}

/// The same `services` query over QUIC and TCP; both must agree.
async fn services(node: &Node, sender: &PeltIdentity) -> io::Result<Vec<String>> {
    let expected = expected(node);
    let quic = query_services_quic(node.quic, sender, &expected).await;
    let tcp = query_services_tcp(&node.tcp.to_string(), sender, &expected).await;
    match (quic, tcp) {
        (Ok(quic), Ok(tcp)) => {
            assert_eq!(quic, tcp);
            Ok(quic.into_iter().map(|entry| entry.name).collect())
        }
        (Err(quic), Err(_)) => Err(quic),
        (quic, tcp) => panic!("transports disagree: {quic:?} / {tcp:?}"),
    }
}

#[tokio::test]
async fn services_show_each_requester_only_its_granted_targets() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (alice, bob) = (generate_identity(), generate_identity());
        let node = node(&[&alice, &bob], Contacts::default()).await;
        {
            let mut st = node.state.lock().await;
            st.published = published();
            st.target_policy = grants(&[
                (&alice, &["127.0.0.1:22", "127.0.0.1:8080"]),
                (&bob, &["127.0.0.1:22"]),
            ]);
        }
        // Published order, filtered per requester by the OPEN decision.
        assert_eq!(services(&node, &alice).await.unwrap(), ["web", "ssh"]);
        assert_eq!(services(&node, &bob).await.unwrap(), ["ssh"]);

        // Revoking a grant hides the entry on the next request.
        node.state.lock().await.target_policy = grants(&[(&alice, &["127.0.0.1:8080"])]);
        assert_eq!(services(&node, &alice).await.unwrap(), ["web"]);
        assert!(services(&node, &bob).await.unwrap().is_empty());

        node.state.lock().await.target_policy = TargetPolicy::LegacyAllow;
        assert_eq!(services(&node, &bob).await.unwrap(), ["web", "ssh", "db"]);
        for policy in [TargetPolicy::Deny, TargetPolicy::Invalid] {
            node.state.lock().await.target_policy = policy;
            assert!(services(&node, &alice).await.unwrap().is_empty());
        }

        // Nothing published: an empty answer, as for nothing granted.
        {
            let mut st = node.state.lock().await;
            st.target_policy = TargetPolicy::LegacyAllow;
            st.published.clear();
        }
        assert!(services(&node, &alice).await.unwrap().is_empty());

        // A services RPC never takes an inbound authority lease.
        let summary = node.state.lock().await.inbound_authority.summary();
        assert_eq!(summary["active"], 0);
        assert_eq!(summary["establishing"], 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn services_refuse_strangers_silver_and_exhausted_rate() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (member, stranger) = (generate_identity(), generate_identity());
        let node = node(&[&member], Contacts::default()).await;
        {
            let mut st = node.state.lock().await;
            st.published = published();
            st.target_policy = TargetPolicy::LegacyAllow;
        }
        // Even LegacyAllow shows nothing to a node outside the Pack.
        assert!(services(&node, &stranger).await.is_err());
        assert_eq!(services(&node, &member).await.unwrap().len(), 3);

        // Shared per-peer bucket with ping and find_node.
        let limiter = node.state.lock().await.rpc_limiter.clone();
        while limiter.allow(&member.fingerprint) {}
        let expected = expected(&node);
        assert!(query_services_quic(node.quic, &member, &expected)
            .await
            .is_err());

        let other = generate_identity();
        node.state.lock().await.peers.push(peer(&other));
        assert_eq!(services(&node, &other).await.unwrap().len(), 3);
        node.state.lock().await.inbound_authority.lock().unwrap();
        assert!(services(&node, &other).await.is_err());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn discover_services_and_add_from_ask_pack_members_now() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (a, b, c) = (
            node(&[], Contacts::default()).await,
            node(&[], Contacts::default()).await,
            node(&[], Contacts::default()).await,
        );
        a.state.lock().await.peers = vec![member("wolf-b", &b), member("wolf-c", &c)];
        // B trusts A; C does not, so C rejects A's queries.
        b.state.lock().await.peers = vec![member("wolf-a", &a)];
        {
            let mut st = b.state.lock().await;
            st.published = published();
            st.target_policy = grants(&[(&a.receiver, &["127.0.0.1:8080"])]);
        }
        c.state.lock().await.published = published();

        let found = operate(&a, "discover.services", serde_json::json!({})).await;
        assert_eq!(found["ok"], true, "{found}");
        assert_eq!(found["result"]["transport"], "quic");
        let queried = found["result"]["queried"].as_array().unwrap();
        assert_eq!(
            queried[0],
            serde_json::json!({"peer": "wolf-b", "status": "ok", "services": [
                {"name": "web", "kind": "http", "target": "127.0.0.1:8080"}
            ]})
        );
        assert_eq!(
            queried[1],
            serde_json::json!({"peer": "wolf-c", "status": "rejected"})
        );

        let one = operate(&a, "discover.services", serde_json::json!({"peer": "wolf-b"})).await;
        assert_eq!(one["result"]["queried"].as_array().unwrap().len(), 1);
        let unknown = operate(&a, "discover.services", serde_json::json!({"peer": "wolf-x"})).await;
        assert_eq!(unknown["error"]["code"], "DISCOVER_INVALID");
        // Over encrypted TCP the answer is the same; a tcp:// Pack address
        // is never dialled as QUIC.
        a.state.lock().await.peers[0].address = format!("tcp://{}", b.tcp);
        let tcp = operate(
            &a,
            "discover.services",
            serde_json::json!({"peer": "wolf-b", "transport": "tcp"}),
        )
        .await;
        assert_eq!(tcp["result"]["queried"][0], queried[0]);
        let quic = operate(&a, "discover.services", serde_json::json!({"peer": "wolf-b"})).await;
        assert_eq!(quic["result"]["queried"][0]["status"], "bad_address");
        // Nothing listening: the dial fails before any handshake, which is
        // reported as unreachable, not rejected.
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        a.state.lock().await.peers[0].address = format!("tcp://{closed}");
        let refused = operate(
            &a,
            "discover.services",
            serde_json::json!({"peer": "wolf-b", "transport": "tcp"}),
        )
        .await;
        assert_eq!(
            refused["result"]["queried"][0],
            serde_json::json!({"peer": "wolf-b", "status": "unreachable"})
        );
        a.state.lock().await.peers[0] = member("wolf-b", &b);

        // --from turns the entry B shows A into an ordinary service.add.
        let request = |name: &str, peer: &str| werewolf_core::protocol::ControlRequest {
            id: "from".into(),
            cmd: "service.add_from".into(),
            args: serde_json::json!({"name": name, "peer": peer}),
        };
        let add = operator::resolve_add_from(request("web", "wolf-b"), &a.state)
            .await
            .unwrap();
        assert_eq!(add.cmd, "service.add");
        assert_eq!(
            add.args,
            serde_json::json!({"name": "web", "peer": "wolf-b", "target": "127.0.0.1:8080", "kind": "http"})
        );
        for (name, peer, code) in [
            ("ssh", "wolf-b", "SERVICE_NOT_FOUND"), // published but not granted to A
            ("web", "wolf-c", "DISCOVER_FAILED"),
            ("web", "wolf-x", "DISCOVER_INVALID"),
            ("", "wolf-b", "SERVICE_INVALID"),
        ] {
            let error = operator::resolve_add_from(request(name, peer), &a.state)
                .await
                .unwrap_err();
            assert_eq!(error.error.unwrap().code, code, "{name} {peer}");
        }
        // Nothing was stored on A by any of this.
        assert!(a.state.lock().await.services.is_empty());

        a.state.lock().await.status.mode = werewolf_core::state::WolfMode::Silver;
        let fenced = operate(&a, "discover.services", serde_json::json!({})).await;
        assert_eq!(fenced["error"]["code"], "SILVER_ACTIVE");
        let error = operator::resolve_add_from(request("web", "wolf-b"), &a.state)
            .await
            .unwrap_err();
        assert_eq!(error.error.unwrap().code, "SILVER_ACTIVE");
    })
    .await
    .unwrap();
}

/// Stage26 decision 5b: a Pack member with a route is queried through its
/// relay with the unchanged RPC, pinned to its own key. The relay needs only
/// its existing Target Policy grant for the member's TCP listener.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discovery_reaches_a_routed_member_through_its_relay() {
    use werewolf_core::route::{Route, RouteHop};
    tokio::time::timeout(Duration::from_secs(60), async {
        let a = node(&[], Contacts::default()).await;
        let b = node(&[&a.receiver], Contacts::default()).await;
        let c = node(&[&a.receiver], Contacts::default()).await;
        let c_listener = c.tcp.to_string();
        {
            let mut st = c.state.lock().await;
            st.published = published();
            st.target_policy = grants(&[(&a.receiver, &["127.0.0.1:8080"])]);
        }
        let route = |hop| Route {
            peer: "wolf-c".into(),
            via: "wolf-b".into(),
            hop,
        };
        {
            let mut st = a.state.lock().await;
            st.peers = vec![
                member("wolf-b", &b),
                PeerRecord {
                    name: "wolf-c".into(),
                    address: format!("tcp://{c_listener}"),
                    ..peer(&c.receiver)
                },
            ];
            st.routes = vec![route(RouteHop::Quic)];
        }
        let ask = || {
            operate(
                &a,
                "discover.services",
                serde_json::json!({"peer": "wolf-c"}),
            )
        };
        let web = serde_json::json!([{"name": "web", "kind": "http", "target": "127.0.0.1:8080"}]);

        // The relay has not granted A C's listener: the relay refuses.
        assert_eq!(ask().await["result"]["queried"][0]["status"], "rejected");

        // With the relay's grant, C answers through it over a QUIC hop.
        b.state.lock().await.target_policy = grants(&[(&a.receiver, &[c_listener.as_str()])]);
        let found = ask().await;
        assert_eq!(found["result"]["queried"][0]["status"], "ok", "{found}");
        assert_eq!(found["result"]["queried"][0]["services"], web);

        // Over a TCP hop to the relay's TCP listener.
        {
            let mut st = a.state.lock().await;
            st.peers[0].address = format!("tcp://{}", b.tcp);
            st.routes = vec![route(RouteHop::Tcp)];
        }
        let found = ask().await;
        assert_eq!(found["result"]["queried"][0]["services"], web, "{found}");

        // Ping through the relay alongside a direct ping to the relay.
        {
            let mut st = a.state.lock().await;
            st.peers[0].address = format!("quic://{}", b.quic);
            st.routes = vec![route(RouteHop::Quic)];
        }
        let refreshed = operate(&a, "discover.refresh", serde_json::json!({})).await;
        let queried = refreshed["result"]["queried"].as_array().unwrap();
        assert!(queried.iter().all(|s| s["status"] == "ok"), "{refreshed}");
        assert_eq!(queried.len(), 2);

        // service add --from works through the relay too.
        let request = werewolf_core::protocol::ControlRequest {
            id: "from".into(),
            cmd: "service.add_from".into(),
            args: serde_json::json!({"name": "web", "peer": "wolf-c"}),
        };
        let add = operator::resolve_add_from(request, &a.state).await.unwrap();
        assert_eq!(add.args["target"], "127.0.0.1:8080");

        // C's own grant still decides what it shows A.
        c.state.lock().await.target_policy = TargetPolicy::Grants(HashMap::new());
        let found = ask().await;
        assert_eq!(
            found["result"]["queried"][0]["services"],
            serde_json::json!([])
        );

        // A route whose relay left the Pack is reported, never dialled
        // directly instead.
        a.state.lock().await.peers.remove(0);
        assert_eq!(ask().await["result"]["queried"][0]["status"], "bad_route");
    })
    .await
    .unwrap();
}
