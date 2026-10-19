//! Operator side of Stage24 discovery: `advertise.json`, the `contacts.json`
//! cache and the `discover.*` control commands. `find` is an iterative
//! (multi-hop) lookup that starts at the Pack and then asks discovered
//! nodes, with TLS pinned to each record's key. Queries, hops and time are
//! bounded because relayed records choose where this node dials. Nothing
//! here changes Pack, Target Policy or authority.
use super::{pack_keys, query_quic, query_tcp, unix_now, Contacts};
use crate::handshake::{self as hs, RpcRequest};
use crate::policy::{select_peer_transport, ExpectedPeerIdentity};
use crate::state::DaemonState;
use crate::transport::FangTransport;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{io, sync::Arc, time::Duration};
use tokio::{
    sync::Mutex,
    task::JoinSet,
    time::{timeout_at, Instant},
};
use werewolf_core::{
    contact::{check_addresses, ContactRecord},
    lookup::MAX_K,
    nodeid::NodeId,
    pack::PeerRecord,
    pelt::{fingerprint_from_public_key_b64, PeltIdentity},
    protocol::{ControlRequest, ControlResponse},
    state::WolfMode,
};

/// Whole refresh or lookup, inside the control request window.
const LOOKUP_BUDGET: Duration = Duration::from_secs(7);
/// Parallel queries (Kademlia α).
const ALPHA: usize = 3;
/// Lookup bounds: hops, nodes asked in total, and time per query.
const MAX_HOPS: usize = 8;
const MAX_QUERIES: usize = 24;
const QUERY_WINDOW: Duration = Duration::from_secs(3);
pub(crate) const ADVERTISE_FILE: &str = "advertise.json";
pub(crate) const CACHE_FILE: &str = "contacts.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdvertiseFile {
    pub(crate) version: u8,
    pub(crate) addresses: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheFile {
    pub(crate) version: u8,
    /// Base64 of `ContactRecord::encode`.
    pub(crate) records: Vec<String>,
}

/// Operator configuration: strict, like `services.json`. An empty list
/// advertises nothing.
pub(crate) fn parse_advertise(data: &[u8]) -> io::Result<Vec<String>> {
    let file: AdvertiseFile = serde_json::from_slice(data).map_err(|_| invalid())?;
    if file.version != 1 {
        return Err(invalid());
    }
    if !file.addresses.is_empty() {
        check_addresses(&file.addresses).map_err(|_| invalid())?;
    }
    Ok(file.addresses)
}

/// The cache is only a hint: a malformed file is ignored as a whole and
/// invalid or expired records one by one, so it can never block startup.
pub(crate) fn parse_cache(data: &[u8], now: u64) -> Vec<ContactRecord> {
    let Ok(file) = serde_json::from_slice::<CacheFile>(data) else {
        eprintln!("ignoring malformed discovery cache {CACHE_FILE}");
        return Vec::new();
    };
    if file.version != 1 {
        eprintln!("ignoring discovery cache {CACHE_FILE} with unknown version");
        return Vec::new();
    }
    let records = file.records.iter().filter_map(|text| {
        let bytes = STANDARD.decode(text).ok()?;
        let record = ContactRecord::decode(&bytes).ok()?;
        record.verify(now).ok().map(|_| record)
    });
    Contacts::default().learn(records, &[0; 32], &[], now)
}

pub(crate) fn cache_file(records: &[ContactRecord]) -> CacheFile {
    CacheFile {
        version: 1,
        records: records
            .iter()
            .map(|record| STANDARD.encode(record.encode()))
            .collect(),
    }
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid discovery state")
}

pub(crate) fn handles(command: &str) -> bool {
    matches!(
        command,
        "discover.show" | "discover.contacts" | "discover.refresh" | "discover.find"
    )
}

pub(crate) async fn handle(req: ControlRequest, state: Arc<Mutex<DaemonState>>) -> ControlResponse {
    match req.cmd.as_str() {
        "discover.show" => {
            let mut st = state.lock().await;
            let Some(pelt) = st.pelt.clone() else {
                return ControlResponse::err(req.id, "NO_PELT", "No local Pelt identity exists");
            };
            st.contacts.refresh_own(&pelt, unix_now());
            let own = st.contacts.own.as_ref().map(|r| view(r, &st.peers));
            ControlResponse::ok(
                req.id,
                json!({
                    "node_id": NodeId::from_public_key(&hs_key(&pelt)).to_hex(),
                    "advertised": st.contacts.advertised,
                    "record": own,
                }),
            )
        }
        "discover.contacts" => {
            let st = state.lock().await;
            let now = unix_now();
            let records: Vec<Value> = st
                .contacts
                .known
                .iter()
                .filter(|record| record.verify(now).is_ok())
                .map(|record| view(record, &st.peers))
                .collect();
            ControlResponse::ok(req.id, json!(records))
        }
        _ => network(req, state).await,
    }
}

/// `discover.refresh` pings every Pack member for its own record;
/// `discover.find` asks the Pack members nearest to a target for it.
async fn network(req: ControlRequest, state: Arc<Mutex<DaemonState>>) -> ControlResponse {
    let transport = match req.args["transport"].as_str().unwrap_or("quic") {
        "quic" => "quic",
        "tcp" => "tcp",
        _ => {
            return ControlResponse::err(
                req.id,
                "DISCOVER_INVALID",
                "transport must be quic or tcp",
            )
        }
    };
    let target = if req.cmd == "discover.find" {
        match parse_target(req.args["target"].as_str().unwrap_or("").trim()) {
            Some(target) => Some(target),
            None => {
                return ControlResponse::err(
                    req.id,
                    "DISCOVER_INVALID",
                    "target must be a public key (base64) or a node ID (64 hex digits)",
                )
            }
        }
    } else {
        None
    };
    let (identity, peers, guard) = {
        let st = state.lock().await;
        if matches!(st.status.mode, WolfMode::Silver) {
            return ControlResponse::err(
                req.id,
                "SILVER_ACTIVE",
                "Silver mode is active. Discovery rejected.",
            );
        }
        let Some(identity) = st.pelt.clone() else {
            return ControlResponse::err(req.id, "NO_PELT", "No local Pelt identity exists");
        };
        let Ok(guard) = st.discovery.clone().try_lock_owned() else {
            return ControlResponse::err(req.id, "DISCOVER_BUSY", "another discovery is running");
        };
        (identity, st.peers.clone(), guard)
    };

    let seeds: Vec<Seed> = peers.iter().filter_map(Seed::from_peer).collect();
    let (outcomes, learned) = match &target {
        Some(target) => lookup(seeds, &peers, &identity, transport, target).await,
        None => run_queries(seeds, &identity, transport, &RpcRequest::Ping).await,
    };

    let cached = remember(&state, &identity, learned.clone()).await;
    drop(guard);

    let st = state.lock().await;
    let mut result = json!({
        "transport": transport,
        "queried": outcomes,
        "cached": cached,
    });
    if let Some(target) = target {
        let mut nearest = Contacts::default().learn(learned, &hs_key(&identity), &[], unix_now());
        nearest.sort_by_key(|record| target.xor_distance(&record.node_id()));
        nearest.truncate(MAX_K);
        let found = nearest.iter().find(|r| r.node_id() == target);
        result["target"] = json!(target.to_hex());
        result["found"] = json!(found.map(|r| view(r, &st.peers)));
        result["nearest"] = json!(nearest
            .iter()
            .map(|r| view(r, &st.peers))
            .collect::<Vec<_>>());
    }
    ControlResponse::ok(req.id, result)
}

/// Persists the merged cache before publishing it ("persist before
/// publish"). Returns false if the new records could not be saved; the
/// previous cache then stays live.
async fn remember(
    state: &Arc<Mutex<DaemonState>>,
    identity: &PeltIdentity,
    learned: Vec<ContactRecord>,
) -> bool {
    let (candidate, degraded) = {
        let st = state.lock().await;
        let candidate = st.contacts.learn(
            learned,
            &hs_key(identity),
            &pack_keys(&st.peers),
            unix_now(),
        );
        if candidate == st.contacts.known {
            return true;
        }
        (candidate, st.storage_degraded)
    };
    if degraded
        || crate::control::persist_discovery_cache(state, &cache_file(&candidate))
            .await
            .is_err()
    {
        return false;
    }
    state.lock().await.contacts.known = candidate;
    true
}

#[derive(Clone)]
struct Seed {
    name: String,
    key: [u8; 32],
    address: String,
    expected: ExpectedPeerIdentity,
}

impl Seed {
    fn from_peer(peer: &PeerRecord) -> Option<Self> {
        let public_key = peer.public_key_b64.clone()?;
        Some(Self {
            name: peer.name.clone(),
            key: hs::binary::<32>(&public_key).ok()?,
            address: peer.address.clone(),
            expected: ExpectedPeerIdentity {
                fingerprint: peer.fingerprint.clone(),
                public_key_b64: Some(public_key),
            },
        })
    }

    /// A node learned from a verified record, dialled at its advertised
    /// address for `transport` (never a scheme-swapped one) and pinned to
    /// the record's key.
    fn from_record(record: &ContactRecord, name: String, transport: &str) -> Option<Self> {
        let address = record
            .addresses
            .iter()
            .find(|address| address.starts_with(&format!("{transport}://")))?
            .clone();
        let public_key_b64 = STANDARD.encode(record.public_key);
        Some(Self {
            name,
            key: record.public_key,
            address,
            expected: ExpectedPeerIdentity {
                fingerprint: fingerprint_from_public_key_b64(&public_key_b64).ok()?,
                public_key_b64: Some(public_key_b64),
            },
        })
    }
}

/// Iterative Kademlia lookup. Each hop asks the `ALPHA` nearest unasked
/// nodes among the `MAX_K` nearest known, Pack members first as seeds, and
/// adds the nodes they return. It stops when the target's record arrives,
/// when the nearest nodes have all been asked, or at `MAX_HOPS`,
/// `MAX_QUERIES` or `LOOKUP_BUDGET`. A node answers only Pack members of its
/// own, so each hop works only through nodes that trust this one.
async fn lookup(
    seeds: Vec<Seed>,
    peers: &[PeerRecord],
    identity: &PeltIdentity,
    transport: &'static str,
    target: &NodeId,
) -> (Vec<Value>, Vec<ContactRecord>) {
    let deadline = Instant::now() + LOOKUP_BUDGET;
    let own = hs_key(identity);
    let request = RpcRequest::FindNode {
        target: target.clone(),
        k: MAX_K as u8,
    };
    let distance = |key: &[u8; 32]| target.xor_distance(&NodeId::from_public_key(key));
    let mut shortlist = seeds;
    let mut asked: Vec<[u8; 32]> = Vec::new();
    let mut statuses = Vec::new();
    let mut learned: Vec<ContactRecord> = Vec::new();
    for hop in 1..=MAX_HOPS {
        shortlist.sort_by_key(|seed| distance(&seed.key));
        let batch: Vec<Seed> = shortlist
            .iter()
            .take(MAX_K)
            .filter(|seed| !asked.contains(&seed.key))
            .take(ALPHA.min(MAX_QUERIES - asked.len()))
            .cloned()
            .collect();
        if batch.is_empty() || Instant::now() >= deadline {
            break;
        }
        let mut running = JoinSet::new();
        for seed in batch {
            asked.push(seed.key);
            let (identity, request) = (identity.clone(), request.clone());
            let until = deadline.min(Instant::now() + QUERY_WINDOW);
            running.spawn(async move {
                let result = timeout_at(until, query(&seed, &identity, transport, &request))
                    .await
                    .unwrap_or(Err("timeout"));
                (seed.name, result)
            });
        }
        while let Some(joined) = running.join_next().await {
            let Ok((name, result)) = joined else { continue };
            let status = match result {
                Ok(records) => {
                    let count = records.len();
                    for record in records {
                        let known = record.public_key == own
                            || shortlist.iter().any(|s| s.key == record.public_key);
                        if !known {
                            let name = label(&record, peers);
                            shortlist.extend(Seed::from_record(&record, name, transport));
                        }
                        learned.push(record);
                    }
                    json!({"peer": name, "hop": hop, "status": "ok", "records": count})
                }
                Err(reason) => json!({"peer": name, "hop": hop, "status": reason}),
            };
            statuses.push(status);
        }
        if learned.iter().any(|record| record.node_id() == *target) {
            break;
        }
    }
    (statuses, learned)
}

/// Pack name if the key is in the Pack, otherwise a short node ID.
fn label(record: &ContactRecord, peers: &[PeerRecord]) -> String {
    let public_key_b64 = STANDARD.encode(record.public_key);
    peers
        .iter()
        .find(|peer| peer.public_key_b64.as_deref() == Some(public_key_b64.as_str()))
        .map_or_else(
            || format!("node:{}", &record.node_id().to_hex()[..16]),
            |peer| peer.name.clone(),
        )
}

/// Runs at most `ALPHA` queries at a time within `LOOKUP_BUDGET`. Every
/// seed gets a status.
async fn run_queries(
    seeds: Vec<Seed>,
    identity: &PeltIdentity,
    transport: &'static str,
    request: &RpcRequest,
) -> (Vec<Value>, Vec<ContactRecord>) {
    let deadline = Instant::now() + LOOKUP_BUDGET;
    let mut statuses: Vec<(String, Value)> = seeds
        .iter()
        .map(|seed| {
            (
                seed.name.clone(),
                json!({"peer": seed.name, "status": "not_queried"}),
            )
        })
        .collect();
    let mut learned = Vec::new();
    let mut pending = seeds.into_iter();
    let mut running = JoinSet::new();
    loop {
        while running.len() < ALPHA {
            let Some(seed) = pending.next() else { break };
            set_status(
                &mut statuses,
                &seed.name,
                json!({"peer": seed.name, "status": "timeout"}),
            );
            let (identity, request) = (identity.clone(), request.clone());
            running.spawn(async move {
                let result = query(&seed, &identity, transport, &request).await;
                (seed.name, result)
            });
        }
        let Ok(Some(joined)) = timeout_at(deadline, running.join_next()).await else {
            break;
        };
        let Ok((name, result)) = joined else { continue };
        let status = match result {
            Ok(records) => {
                let status = json!({"peer": name, "status": "ok", "records": records.len()});
                learned.extend(records);
                status
            }
            Err(reason) => json!({"peer": name, "status": reason}),
        };
        set_status(&mut statuses, &name, status);
    }
    running.abort_all();
    (
        statuses.into_iter().map(|(_, status)| status).collect(),
        learned,
    )
}

fn set_status(statuses: &mut [(String, Value)], name: &str, status: Value) {
    if let Some(entry) = statuses.iter_mut().find(|(peer, _)| peer == name) {
        entry.1 = status;
    }
}

async fn query(
    seed: &Seed,
    identity: &PeltIdentity,
    transport: &str,
    request: &RpcRequest,
) -> Result<Vec<ContactRecord>, &'static str> {
    match select_peer_transport(transport, &seed.address) {
        Ok(FangTransport::Quic(address)) => {
            let address = tokio::net::lookup_host(address.as_str())
                .await
                .ok()
                .and_then(|mut addresses| addresses.next())
                .ok_or("bad_address")?;
            query_quic(address, identity, &seed.expected, request)
                .await
                .map_err(|_| "rejected")
        }
        Ok(FangTransport::Tcp(address)) => query_tcp(&address, identity, &seed.expected, request)
            .await
            .map_err(|_| "rejected"),
        Err(_) => Err("bad_address"),
    }
}

/// A node ID (64 hex digits) or a public key (base64, 32 bytes).
fn parse_target(text: &str) -> Option<NodeId> {
    if text.len() == 64 {
        if let Ok(id) = NodeId::from_hex(text) {
            return Some(id);
        }
    }
    hs::binary::<32>(text)
        .ok()
        .map(|key| NodeId::from_public_key(&key))
}

fn hs_key(identity: &PeltIdentity) -> [u8; 32] {
    hs::binary::<32>(&identity.public_key_b64).unwrap_or([0; 32])
}

/// Operator view of a record. `pack_name` is set when the key is already in
/// the Pack; otherwise trusting the node is a separate `pack add`.
pub(crate) fn view(record: &ContactRecord, peers: &[PeerRecord]) -> Value {
    let public_key_b64 = STANDARD.encode(record.public_key);
    let pack_name = peers
        .iter()
        .find(|peer| peer.public_key_b64.as_deref() == Some(public_key_b64.as_str()))
        .map(|peer| peer.name.clone());
    json!({
        "node_id": record.node_id().to_hex(),
        "public_key_b64": public_key_b64,
        "fingerprint": fingerprint_from_public_key_b64(&public_key_b64).ok(),
        "addresses": record.addresses,
        "seq": record.seq,
        "expires_at": record.expires_at,
        "pack_name": pack_name,
    })
}

#[cfg(test)]
mod tests;
