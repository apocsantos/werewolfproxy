//! Stage24 discovery (`docs/STAGE24_FINDNODE_RPC_WIRE_SPEC.md`): what a
//! receiver answers to FindNode/Ping, the per-peer RPC rate limit, and the
//! requester side. Discovery is read-only and never grants trust: nothing
//! here touches Pack, Target Policy, authority leases or persisted state.
use crate::admission::{HandshakePermit, QuicReplayV3};
use crate::handshake::{self as hs, RpcRequest};
use crate::policy::ExpectedPeerIdentity;
use crate::state::DaemonState;
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
use werewolf_core::{contact::ContactRecord, pelt::PeltIdentity};

/// Per-peer RPC token bucket: burst and refill per second (spec "Limits").
const RPC_BURST: f64 = 16.0;
const RPC_REFILL_PER_SECOND: f64 = 4.0;

/// Records this node can answer with. Step 3 fills them (advertised address,
/// cache, lookups); until then a receiver answers with empty results.
#[derive(Clone, Debug, Default)]
pub(crate) struct Contacts {
    pub(crate) own: Option<ContactRecord>,
    pub(crate) known: Vec<ContactRecord>,
}

impl Contacts {
    /// Ping: our own record. FindNode: up to `k` records closest to the
    /// target, excluding the requester, each re-verified against `now`.
    fn answer(&self, request: &RpcRequest, requester: &[u8; 32], now: u64) -> Vec<ContactRecord> {
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
                    .filter(|record| record.public_key != *requester)
                    .collect();
                candidates.sort_by_key(|record| target.xor_distance(&record.node_id()));
                candidates
                    .into_iter()
                    .filter(|record| record.verify(now).is_ok())
                    .take(*k as usize)
                    .cloned()
                    .collect()
            }
        }
    }
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

/// Snapshot taken once per RPC under the state lock.
struct Responder {
    receiver: PeltIdentity,
    contacts: Contacts,
    limiter: Arc<RpcLimiter>,
    locked: bool,
}

fn responder(st: &DaemonState) -> io::Result<Responder> {
    Ok(Responder {
        receiver: st.pelt.clone().ok_or_else(hs::rejected)?,
        contacts: st.contacts.clone(),
        limiter: st.rpc_limiter.clone(),
        locked: st.inbound_authority.is_locked(),
    })
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
        let st = state.lock().await;
        let key = st
            .peers
            .iter()
            .find(|p| p.fingerprint == rpc.sender_fingerprint)
            .and_then(|p| p.public_key_b64.clone())
            .ok_or_else(hs::rejected)?;
        (key, responder(&st)?)
    };
    hs::require(!responder.locked)?;
    hs::require(rpc.receiver_fingerprint == responder.receiver.fingerprint)?;
    hs::verify(&sender_key, &retained, &rpc.signature)?;
    hs::require(Instant::now() < started + hs::READ_WINDOW)?;
    permit.authenticated(&rpc.sender_fingerprint)?;
    replay.reserve(&rpc.sender_fingerprint, &rpc.nonce)?;
    hs::require(responder.limiter.allow(&rpc.sender_fingerprint))?;
    let records =
        responder
            .contacts
            .answer(&rpc.request()?, &hs::binary::<32>(&sender_key)?, unix_now());
    let result = fit(records, |records| {
        hs::QuicRpcResult::new(rpc, &retained, &responder.receiver, records)
    })?;
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
        let st = state.lock().await;
        hs::require(st.peers.iter().any(|p| {
            p.fingerprint == rpc.sender_fingerprint
                && p.public_key_b64.as_deref() == Some(rpc.sender_pubkey.as_str())
        }))?;
        responder(&st)?
    };
    hs::require(!responder.locked)?;
    context.check_binding(&rpc.challenge, &rpc.receiver_fingerprint)?;
    hs::verify(&rpc.sender_pubkey, &retained, &rpc.signature)?;
    permit.authenticated(&rpc.sender_fingerprint)?;
    context.consume_binding(&rpc.challenge, &rpc.receiver_fingerprint)?;
    hs::require(responder.limiter.allow(&rpc.sender_fingerprint))?;
    let records = responder
        .contacts
        .answer(&rpc.request()?, &requester, unix_now());
    let result = fit(records, |records| {
        hs::TcpRpcResult::new(rpc, &retained, &context.receiver, records)
    })?;
    hs::write(
        stream,
        &result,
        hs::RESULT_LIMIT,
        started + hs::SERVER_WINDOW,
    )
    .await
}

/// Requester side over QUIC: one connection, one RPC stream. Returns the
/// individually verified records. Step 3 wires this to `werewolfctl`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn query_quic(
    address: SocketAddr,
    identity: &PeltIdentity,
    expected: &ExpectedPeerIdentity,
    request: &RpcRequest,
) -> io::Result<Vec<ContactRecord>> {
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
        result.verify(&rpc, &retained, unix_now())
    })
    .await
    .map_err(|_| hs::rejected())
    .and_then(|result| result);
    connection.close(0u32.into(), b"");
    result
}

/// Requester side over encrypted TCP: one TLS connection, one RPC.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn query_tcp(
    peer_addr: &str,
    identity: &PeltIdentity,
    expected: &ExpectedPeerIdentity,
    request: &RpcRequest,
) -> io::Result<Vec<ContactRecord>> {
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
        result.verify(&rpc, &retained, unix_now())
    })
    .await
    .map_err(|_| hs::rejected())?
}

#[cfg(test)]
mod tests;
