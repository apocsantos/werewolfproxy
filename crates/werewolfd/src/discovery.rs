//! Stage24 discovery (`docs/STAGE24_FINDNODE_RPC_WIRE_SPEC.md`) and the
//! Stage25 `services` op (`docs/STAGE25_SERVICES_RPC_WIRE_SPEC.md`): what a
//! receiver answers, the per-peer RPC rate limit, and the requester side.
//! Discovery is read-only and never grants trust: nothing here changes Pack,
//! Target Policy, authority leases or persisted state. `services` only reads
//! Target Policy, to show a requester the targets it may already open.
use crate::admission::{HandshakePermit, QuicReplayV3};
use crate::handshake::{self as hs, RpcRequest};
use crate::policy::ExpectedPeerIdentity;
use crate::state::DaemonState;
use crate::target_policy::TargetPolicy;
use serde::Serialize;
use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::Arc,
    time::{Instant as StdInstant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::Mutex,
    time::{timeout_at, Instant},
};
use werewolf_core::{
    contact::{ContactRecord, MAX_LIFETIME_SECS},
    pack::PeerRecord,
    pelt::PeltIdentity,
    published::PublishedService,
};

/// Per-peer RPC token bucket: burst and refill per second (spec "Limits").
const RPC_BURST: f64 = 16.0;
const RPC_REFILL_PER_SECOND: f64 = 4.0;
/// Our record is re-signed once less than this much validity is left.
const RESIGN_MARGIN_SECS: u64 = 24 * 60 * 60;
/// Most records kept in `contacts.json`.
pub(crate) const CACHE_LIMIT: usize = 256;

/// Our advertised addresses and signed record, and the contact cache.
#[derive(Clone, Debug, Default)]
pub(crate) struct Contacts {
    /// Operator-chosen addresses (`advertise.json`); empty advertises nothing.
    pub(crate) advertised: Vec<String>,
    pub(crate) own: Option<ContactRecord>,
    /// Non-authoritative cache (`contacts.json`) of records verified when
    /// learned; every use verifies them again.
    pub(crate) known: Vec<ContactRecord>,
}

impl Contacts {
    /// Ping: our own record. FindNode: up to `k` records of our own Pack
    /// members closest to the target, excluding the requester, each
    /// re-verified against `now`. Records of other nodes in the cache are
    /// never relayed, so a node vouches only for nodes it trusts itself.
    fn answer(
        &self,
        request: &RpcRequest,
        requester: &[u8; 32],
        pack: &[[u8; 32]],
        now: u64,
    ) -> Vec<ContactRecord> {
        match request {
            RpcRequest::Ping => self
                .own
                .iter()
                .filter(|record| record.verify(now).is_ok())
                .cloned()
                .collect(),
            RpcRequest::FindNode { target, k } => {
                let mut candidates: Vec<&ContactRecord> = self
                    .known
                    .iter()
                    .filter(|record| {
                        record.public_key != *requester && pack.contains(&record.public_key)
                    })
                    .collect();
                candidates.sort_by_key(|record| target.xor_distance(&record.node_id()));
                candidates
                    .into_iter()
                    .filter(|record| record.verify(now).is_ok())
                    .take(*k as usize)
                    .cloned()
                    .collect()
            }
            // Answered from published services, never from contact records.
            RpcRequest::Services => Vec::new(),
        }
    }

    /// Re-signs our record when the addresses changed or less than a day of
    /// validity is left. `seq` follows the clock, so it keeps increasing
    /// across restarts without being persisted.
    pub(crate) fn refresh_own(&mut self, identity: &PeltIdentity, now: u64) {
        if self.advertised.is_empty() {
            self.own = None;
            return;
        }
        let current = self.own.as_ref().is_some_and(|record| {
            record.addresses == self.advertised
                && record.expires_at > now + RESIGN_MARGIN_SECS
                && record.verify(now).is_ok()
        });
        if current {
            return;
        }
        let seq = self
            .own
            .as_ref()
            .map_or(now, |record| now.max(record.seq.saturating_add(1)));
        self.own = ContactRecord::sign(
            identity,
            self.advertised.clone(),
            seq,
            now + MAX_LIFETIME_SECS,
            now,
        )
        .ok();
    }

    /// Merges verified records into the cache: drops expired ones and our
    /// own, keeps the highest `seq` per node and at most `CACHE_LIMIT`
    /// records, evicting non-Pack records and the earliest expiry first.
    pub(crate) fn learn(
        &self,
        records: impl IntoIterator<Item = ContactRecord>,
        own_key: &[u8; 32],
        pack: &[[u8; 32]],
        now: u64,
    ) -> Vec<ContactRecord> {
        let mut merged: Vec<ContactRecord> = Vec::new();
        for record in self.known.iter().cloned().chain(records) {
            if record.public_key == *own_key || record.verify(now).is_err() {
                continue;
            }
            match merged
                .iter_mut()
                .find(|kept| kept.public_key == record.public_key)
            {
                Some(kept) if record.supersedes(kept) => *kept = record,
                Some(_) => {}
                None => merged.push(record),
            }
        }
        merged.sort_by_key(|record| {
            (
                !pack.contains(&record.public_key),
                std::cmp::Reverse(record.expires_at),
            )
        });
        merged.truncate(CACHE_LIMIT);
        merged
    }
}

/// Public keys of Pack members that carry one.
pub(crate) fn pack_keys(peers: &[PeerRecord]) -> Vec<[u8; 32]> {
    peers
        .iter()
        .filter_map(|peer| peer.public_key_b64.as_deref())
        .filter_map(|key| hs::binary::<32>(key).ok())
        .collect()
}

#[derive(Debug, Default)]
pub(crate) struct RpcLimiter {
    buckets: std::sync::Mutex<HashMap<String, (f64, StdInstant)>>,
}

impl RpcLimiter {
    /// Called only after the sender is authenticated as a Pack member, so the
    /// map holds at most one entry per Pack fingerprint.
    fn allow(&self, fingerprint: &str) -> bool {
        self.allow_at(fingerprint, StdInstant::now())
    }

    fn allow_at(&self, fingerprint: &str, now: StdInstant) -> bool {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let (tokens, last) = buckets
            .entry(fingerprint.to_string())
            .or_insert((RPC_BURST, now));
        let elapsed = now.saturating_duration_since(*last).as_secs_f64();
        *tokens = (*tokens + elapsed * RPC_REFILL_PER_SECOND).min(RPC_BURST);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Builds a signed result, dropping the farthest records until the line
/// fits `RESULT_LIMIT`.
fn fit<T: Serialize>(
    mut records: Vec<ContactRecord>,
    build: impl Fn(&[ContactRecord]) -> io::Result<T>,
) -> io::Result<T> {
    loop {
        let result = build(&records)?;
        let len = serde_json::to_vec(&result)
            .map_err(|_| hs::rejected())?
            .len()
            + 1;
        if len <= hs::RESULT_LIMIT {
            return Ok(result);
        }
        hs::require(records.pop().is_some())?;
    }
}

/// Snapshot taken once per RPC under the state lock. Re-signing our own
/// record here changes memory only; an RPC never writes persisted state.
struct Responder {
    receiver: PeltIdentity,
    contacts: Contacts,
    pack: Vec<[u8; 32]>,
    published: Vec<PublishedService>,
    policy: TargetPolicy,
    limiter: Arc<RpcLimiter>,
    locked: bool,
}

fn responder(st: &mut DaemonState) -> io::Result<Responder> {
    let receiver = st.pelt.clone().ok_or_else(hs::rejected)?;
    st.contacts.refresh_own(&receiver, unix_now());
    Ok(Responder {
        receiver,
        contacts: st.contacts.clone(),
        pack: pack_keys(&st.peers),
        published: st.published.clone(),
        policy: st.target_policy.clone(),
        limiter: st.rpc_limiter.clone(),
        locked: st.inbound_authority.is_locked(),
    })
}

/// Published services this requester could open right now: the same Target
/// Policy decision OPEN applies to that sender and target, in published
/// order. Published targets are literal, so this never resolves DNS.
async fn visible_services(
    published: &[PublishedService],
    policy: &TargetPolicy,
    requester_fingerprint: &str,
) -> Vec<PublishedService> {
    let mut visible = Vec::new();
    for entry in published {
        if entry.target.parse::<SocketAddr>().is_ok()
            && crate::target_policy::authorize(policy, requester_fingerprint, &entry.target)
                .await
                .is_ok()
        {
            visible.push(entry.clone());
        }
    }
    visible
}

/// Receiver side of a QUIC RPC, after the line was dispatched on `cmd`.
/// Order follows the spec: Silver, binding, Pack key, signature, peer
/// admission, replay, rate limit, answer.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn answer_quic<W: AsyncWrite + Unpin>(
    send: &mut W,
    rpc: &hs::QuicRpc,
    binding: &[u8; 32],
    state: &Arc<Mutex<DaemonState>>,
    permit: &mut HandshakePermit,
    replay: &QuicReplayV3,
    started: Instant,
) -> io::Result<()> {
    let retained = rpc.transcript(binding)?;
    let (sender_key, responder) = {
        let mut st = state.lock().await;
        let key = st
            .peers
            .iter()
            .find(|p| p.fingerprint == rpc.sender_fingerprint)
            .and_then(|p| p.public_key_b64.clone())
            .ok_or_else(hs::rejected)?;
        (key, responder(&mut st)?)
    };
    hs::require(!responder.locked)?;
    hs::require(rpc.receiver_fingerprint == responder.receiver.fingerprint)?;
    hs::verify(&sender_key, &retained, &rpc.signature)?;
    hs::require(Instant::now() < started + hs::READ_WINDOW)?;
    permit.authenticated(&rpc.sender_fingerprint)?;
    replay.reserve(&rpc.sender_fingerprint, &rpc.nonce)?;
    hs::require(responder.limiter.allow(&rpc.sender_fingerprint))?;
    let request = rpc.request()?;
    let result = if request == RpcRequest::Services {
        let entries = visible_services(
            &responder.published,
            &responder.policy,
            &rpc.sender_fingerprint,
        )
        .await;
        // Never truncated: `write` rejects a line over `RESULT_LIMIT`.
        hs::QuicRpcResult::new(rpc, &retained, &responder.receiver, &entries)?
    } else {
        let records = responder.contacts.answer(
            &request,
            &hs::binary::<32>(&sender_key)?,
            &responder.pack,
            unix_now(),
        );
        fit(records, |records| {
            hs::QuicRpcResult::new(rpc, &retained, &responder.receiver, records)
        })?
    };
    hs::write(send, &result, hs::RESULT_LIMIT, started + hs::SERVER_WINDOW).await
}

/// Receiver side of a TCP RPC, after the unchanged challenge. The sender's
/// claimed key must equal its Pack key, not only match its fingerprint.
pub(crate) async fn answer_tcp<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    rpc: &hs::TcpRpc,
    context: &mut hs::TcpChallengeContext,
    state: &Arc<Mutex<DaemonState>>,
    permit: &mut HandshakePermit,
    started: Instant,
) -> io::Result<()> {
    let retained = rpc.transcript()?;
    let requester = hs::tcp_sender_key(rpc)?;
    let responder = {
        let mut st = state.lock().await;
        hs::require(st.peers.iter().any(|p| {
            p.fingerprint == rpc.sender_fingerprint
                && p.public_key_b64.as_deref() == Some(rpc.sender_pubkey.as_str())
        }))?;
        responder(&mut st)?
    };
    hs::require(!responder.locked)?;
    context.check_binding(&rpc.challenge, &rpc.receiver_fingerprint)?;
    hs::verify(&rpc.sender_pubkey, &retained, &rpc.signature)?;
    permit.authenticated(&rpc.sender_fingerprint)?;
    context.consume_binding(&rpc.challenge, &rpc.receiver_fingerprint)?;
    hs::require(responder.limiter.allow(&rpc.sender_fingerprint))?;
    let request = rpc.request()?;
    let result = if request == RpcRequest::Services {
        let entries = visible_services(
            &responder.published,
            &responder.policy,
            &rpc.sender_fingerprint,
        )
        .await;
        hs::TcpRpcResult::new(rpc, &retained, &context.receiver, &entries)?
    } else {
        let records = responder
            .contacts
            .answer(&request, &requester, &responder.pack, unix_now());
        fit(records, |records| {
            hs::TcpRpcResult::new(rpc, &retained, &context.receiver, records)
        })?
    };
    hs::write(
        stream,
        &result,
        hs::RESULT_LIMIT,
        started + hs::SERVER_WINDOW,
    )
    .await
}

/// Requester side over QUIC: one connection, one RPC stream. Returns the
/// individually verified records.
pub(crate) async fn query_quic(
    address: SocketAddr,
    identity: &PeltIdentity,
    expected: &ExpectedPeerIdentity,
    request: &RpcRequest,
) -> io::Result<Vec<ContactRecord>> {
    rpc_quic(
        address,
        identity,
        expected,
        request,
        |result, rpc, retained| result.verify(rpc, retained, unix_now()),
    )
    .await
}

/// `services` over QUIC: the responder's published services visible to us.
/// Any bad entry rejects the whole answer.
#[cfg_attr(not(test), expect(dead_code, reason = "used by Stage25 step 3"))]
pub(crate) async fn query_services_quic(
    address: SocketAddr,
    identity: &PeltIdentity,
    expected: &ExpectedPeerIdentity,
) -> io::Result<Vec<PublishedService>> {
    rpc_quic(
        address,
        identity,
        expected,
        &RpcRequest::Services,
        |result, rpc, retained| result.verify_services(rpc, retained),
    )
    .await
}

async fn rpc_quic<T>(
    address: SocketAddr,
    identity: &PeltIdentity,
    expected: &ExpectedPeerIdentity,
    request: &RpcRequest,
    check: impl FnOnce(&hs::QuicRpcResult, &hs::QuicRpc, &[u8]) -> io::Result<T>,
) -> io::Result<T> {
    let endpoint = crate::quic_fang::client_endpoint(expected)?;
    let connecting = endpoint
        .connect(address, &address.ip().to_string())
        .map_err(|_| hs::rejected())?;
    let connection = tokio::time::timeout(hs::READ_WINDOW, connecting)
        .await
        .map_err(|_| hs::rejected())?
        .map_err(|_| hs::rejected())?;
    let deadline = Instant::now() + hs::CLIENT_WINDOW;
    let result = timeout_at(deadline, async {
        let binding = hs::quic_binding(&connection)?;
        let (mut send, mut recv) = connection.open_bi().await.map_err(|_| hs::rejected())?;
        let rpc = hs::QuicRpc::new(identity, &expected.fingerprint, request, &binding)?;
        let retained = rpc.transcript(&binding)?;
        hs::write(&mut send, &rpc, hs::MESSAGE_LIMIT, deadline).await?;
        let _ = send.finish();
        let result: hs::QuicRpcResult = hs::read(&mut recv, hs::RESULT_LIMIT, deadline).await?;
        check(&result, &rpc, &retained)
    })
    .await
    .map_err(|_| hs::rejected())
    .and_then(|result| result);
    connection.close(0u32.into(), b"");
    result
}

/// Requester side over encrypted TCP: one TLS connection, one RPC.
pub(crate) async fn query_tcp(
    peer_addr: &str,
    identity: &PeltIdentity,
    expected: &ExpectedPeerIdentity,
    request: &RpcRequest,
) -> io::Result<Vec<ContactRecord>> {
    rpc_tcp(
        peer_addr,
        identity,
        expected,
        request,
        |result, rpc, retained| result.verify(rpc, retained, unix_now()),
    )
    .await
}

/// `services` over encrypted TCP (see `query_services_quic`).
#[cfg_attr(not(test), expect(dead_code, reason = "used by Stage25 step 3"))]
pub(crate) async fn query_services_tcp(
    peer_addr: &str,
    identity: &PeltIdentity,
    expected: &ExpectedPeerIdentity,
) -> io::Result<Vec<PublishedService>> {
    rpc_tcp(
        peer_addr,
        identity,
        expected,
        &RpcRequest::Services,
        |result, rpc, retained| result.verify_services(rpc, retained),
    )
    .await
}

async fn rpc_tcp<T>(
    peer_addr: &str,
    identity: &PeltIdentity,
    expected: &ExpectedPeerIdentity,
    request: &RpcRequest,
    check: impl FnOnce(&hs::TcpRpcResult, &hs::TcpRpc, &[u8]) -> io::Result<T>,
) -> io::Result<T> {
    let mut stream =
        crate::transport::tcp_encrypted::connect_authenticated_tcp(peer_addr, expected).await?;
    let deadline = Instant::now() + hs::CLIENT_WINDOW;
    timeout_at(deadline, async {
        let challenge: hs::TcpChallenge =
            hs::read(&mut stream, hs::CHALLENGE_LIMIT, deadline).await?;
        challenge.validate()?;
        hs::require(challenge.receiver_fingerprint == expected.fingerprint)?;
        let rpc = hs::TcpRpc::new(identity, &challenge, request)?;
        let retained = rpc.transcript()?;
        hs::write(&mut stream, &rpc, hs::MESSAGE_LIMIT, deadline).await?;
        let result: hs::TcpRpcResult = hs::read(&mut stream, hs::RESULT_LIMIT, deadline).await?;
        check(&result, &rpc, &retained)
    })
    .await
    .map_err(|_| hs::rejected())?
}

pub(crate) mod operator;

#[cfg(test)]
mod tests;
