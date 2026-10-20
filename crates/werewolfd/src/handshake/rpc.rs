//! Stage24 FindNode/Ping RPC messages (`docs/STAGE24_FINDNODE_RPC_WIRE_SPEC.md`)
//! and the Stage25 `services` op (`docs/STAGE25_SERVICES_RPC_WIRE_SPEC.md`).
//! These share the OPEN position on each transport but use their own `/v1`
//! transcript domains, so no RPC signature verifies as an OPEN or ACK.
use super::{
    binary, fingerprint, hex, public_matches, random, rejected, require, sign, transcript, unhex,
    QUIC, TCP,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::io;
use werewolf_core::{
    contact::ContactRecord,
    nodeid::NodeId,
    pelt::PeltIdentity,
    published::{self, PublishedService},
};

pub(crate) const QUIC_RPC: &str = "fang.quic.rpc";
pub(crate) const TCP_RPC: &str = "fang.tcp.rpc";
const QUIC_RESULT: &str = "fang.quic.rpc.result";
const TCP_RESULT: &str = "fang.tcp.rpc.result";
const PING: &str = "ping";
const FIND_NODE: &str = "find_node";
const SERVICES: &str = "services";
/// Largest RPC result line; responders drop the farthest records to fit.
pub(crate) const RESULT_LIMIT: usize = 32 * 1024;
const MAX_K: u8 = werewolf_core::lookup::MAX_K as u8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RpcRequest {
    Ping,
    FindNode {
        target: NodeId,
        k: u8,
    },
    /// The responder's own published services visible to the requester.
    Services,
}

/// What a result's `records` carry: contact records for `ping` and
/// `find_node`, published service entries for `services`.
pub(crate) trait RpcEntry {
    fn encode(&self) -> Vec<u8>;
}

impl RpcEntry for ContactRecord {
    fn encode(&self) -> Vec<u8> {
        ContactRecord::encode(self)
    }
}

impl RpcEntry for PublishedService {
    fn encode(&self) -> Vec<u8> {
        PublishedService::encode(self)
    }
}

impl RpcRequest {
    fn fields(&self) -> (String, String, u8) {
        match self {
            Self::Ping => (PING.into(), String::new(), 0),
            Self::FindNode { target, k } => (FIND_NODE.into(), target.to_hex(), *k),
            Self::Services => (SERVICES.into(), String::new(), 0),
        }
    }

    fn parse(op: &str, target: &str, k: u8) -> io::Result<Self> {
        match op {
            PING => {
                require(target.is_empty() && k == 0)?;
                Ok(Self::Ping)
            }
            FIND_NODE => {
                require((1..=MAX_K).contains(&k))?;
                Ok(Self::FindNode {
                    target: NodeId(unhex::<32>(target)?),
                    k,
                })
            }
            SERVICES => {
                require(target.is_empty() && k == 0)?;
                Ok(Self::Services)
            }
            _ => Err(rejected()),
        }
    }

    /// Records a result for this request may carry.
    pub(crate) fn max_records(&self) -> usize {
        match self {
            Self::Ping => 1,
            Self::FindNode { k, .. } => *k as usize,
            Self::Services => published::MAX_PUBLISHED,
        }
    }

    fn target_bytes(&self) -> Vec<u8> {
        match self {
            Self::Ping | Self::Services => Vec::new(),
            Self::FindNode { target, .. } => target.0.to_vec(),
        }
    }

    fn k(&self) -> u8 {
        self.fields().2
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuicRpc {
    pub(crate) cmd: String,
    pub(crate) protocol: String,
    pub(crate) sender_fingerprint: String,
    pub(crate) receiver_fingerprint: String,
    pub(crate) nonce: String,
    pub(crate) op: String,
    pub(crate) target: String,
    pub(crate) k: u8,
    pub(crate) signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuicRpcResult {
    pub(crate) cmd: String,
    pub(crate) ok: bool,
    pub(crate) protocol: String,
    pub(crate) receiver_pubkey: String,
    pub(crate) receiver_fingerprint: String,
    pub(crate) sender_fingerprint: String,
    pub(crate) nonce: String,
    pub(crate) op: String,
    pub(crate) records: Vec<String>,
    pub(crate) signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TcpRpc {
    pub(crate) cmd: String,
    pub(crate) protocol: String,
    pub(crate) sender_pubkey: String,
    pub(crate) sender_fingerprint: String,
    pub(crate) receiver_fingerprint: String,
    pub(crate) challenge: String,
    pub(crate) nonce: String,
    pub(crate) op: String,
    pub(crate) target: String,
    pub(crate) k: u8,
    pub(crate) signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TcpRpcResult {
    pub(crate) cmd: String,
    pub(crate) ok: bool,
    pub(crate) protocol: String,
    pub(crate) receiver_pubkey: String,
    pub(crate) receiver_fingerprint: String,
    pub(crate) sender_fingerprint: String,
    pub(crate) challenge: String,
    pub(crate) nonce: String,
    pub(crate) op: String,
    pub(crate) records: Vec<String>,
    pub(crate) signature: String,
}

/// A result's transcript, its raw record bytes and the receiver key, after
/// the envelope checks and before signature verification.
type CheckedResult = (Vec<u8>, (Vec<Vec<u8>>, [u8; 32]));

/// Canonical base64 of encoded records; any non-canonical text is rejected.
fn record_bytes(records: &[String], max: usize) -> io::Result<Vec<Vec<u8>>> {
    require(records.len() <= max)?;
    records
        .iter()
        .map(|text| {
            let bytes = STANDARD.decode(text).map_err(|_| rejected())?;
            require(STANDARD.encode(&bytes) == *text)?;
            Ok(bytes)
        })
        .collect()
}

fn result_transcript(
    domain: &str,
    protocol: &str,
    retained: &[u8],
    receiver_pubkey: &[u8; 32],
    records: &[Vec<u8>],
) -> Vec<u8> {
    let count = [records.len() as u8];
    let mut fields: Vec<&[u8]> = vec![
        protocol.as_bytes(),
        &[1u8][..],
        retained,
        &receiver_pubkey[..],
        &count[..],
    ];
    fields.extend(records.iter().map(Vec::as_slice));
    transcript(domain, &fields)
}

/// Decodes records from a verified result. Each record is verified on its
/// own; invalid or expired ones are dropped, as the spec allows clock skew.
fn accepted_records(
    raw: Vec<Vec<u8>>,
    request: &RpcRequest,
    receiver_pubkey: &[u8; 32],
    now: u64,
) -> io::Result<Vec<ContactRecord>> {
    require(*request != RpcRequest::Services)?;
    Ok(raw
        .iter()
        .filter_map(|bytes| ContactRecord::decode(bytes).ok())
        .filter(|record| record.verify(now).is_ok())
        .filter(|record| *request != RpcRequest::Ping || record.public_key == *receiver_pubkey)
        .collect())
}

/// Decodes service entries from a verified `services` result. Entries share
/// the result signature instead of carrying their own, so one bad entry
/// means the responder is faulty or hostile and rejects the whole result.
fn accepted_services(raw: Vec<Vec<u8>>, request: &RpcRequest) -> io::Result<Vec<PublishedService>> {
    require(*request == RpcRequest::Services)?;
    let entries = raw
        .iter()
        .map(|bytes| PublishedService::decode(bytes).map_err(|_| rejected()))
        .collect::<io::Result<Vec<_>>>()?;
    published::validate_published(&entries).map_err(|_| rejected())?;
    Ok(entries)
}

impl QuicRpc {
    pub(crate) fn request(&self) -> io::Result<RpcRequest> {
        require(self.cmd == QUIC_RPC && self.protocol == QUIC)?;
        RpcRequest::parse(&self.op, &self.target, self.k)
    }

    pub(crate) fn transcript(&self, binding: &[u8; 32]) -> io::Result<Vec<u8>> {
        let request = self.request()?;
        fingerprint(&self.sender_fingerprint)?;
        fingerprint(&self.receiver_fingerprint)?;
        Ok(transcript(
            "fang.quic.rpc/v1",
            &[
                QUIC.as_bytes(),
                self.sender_fingerprint.as_bytes(),
                self.receiver_fingerprint.as_bytes(),
                &unhex::<16>(&self.nonce)?,
                self.op.as_bytes(),
                &request.target_bytes(),
                &[request.k()],
                binding,
            ],
        ))
    }

    pub(crate) fn new(
        sender: &PeltIdentity,
        receiver: &str,
        request: &RpcRequest,
        binding: &[u8; 32],
    ) -> io::Result<Self> {
        let (op, target, k) = request.fields();
        let mut value = Self {
            cmd: QUIC_RPC.into(),
            protocol: QUIC.into(),
            sender_fingerprint: sender.fingerprint.clone(),
            receiver_fingerprint: receiver.into(),
            nonce: hex(&random::<16>()?),
            op,
            target,
            k,
            signature: String::new(),
        };
        value.signature = sign(sender, &value.transcript(binding)?)?;
        Ok(value)
    }
}

impl QuicRpcResult {
    pub(crate) fn new<R: RpcEntry>(
        rpc: &QuicRpc,
        retained: &[u8],
        receiver: &PeltIdentity,
        records: &[R],
    ) -> io::Result<Self> {
        let mut value = Self {
            cmd: QUIC_RESULT.into(),
            ok: true,
            protocol: QUIC.into(),
            receiver_pubkey: receiver.public_key_b64.clone(),
            receiver_fingerprint: receiver.fingerprint.clone(),
            sender_fingerprint: rpc.sender_fingerprint.clone(),
            nonce: rpc.nonce.clone(),
            op: rpc.op.clone(),
            records: records
                .iter()
                .map(|r| STANDARD.encode(RpcEntry::encode(r)))
                .collect(),
            signature: String::new(),
        };
        let (transcript, _) = value.transcript(rpc, retained)?;
        value.signature = sign(receiver, &transcript)?;
        Ok(value)
    }

    fn transcript(&self, rpc: &QuicRpc, retained: &[u8]) -> io::Result<CheckedResult> {
        require(
            self.cmd == QUIC_RESULT
                && self.protocol == QUIC
                && self.ok
                && self.receiver_fingerprint == rpc.receiver_fingerprint
                && self.sender_fingerprint == rpc.sender_fingerprint
                && self.nonce == rpc.nonce
                && self.op == rpc.op,
        )?;
        let receiver = public_matches(&self.receiver_pubkey, &self.receiver_fingerprint)?;
        let raw = record_bytes(&self.records, rpc.request()?.max_records())?;
        let transcript =
            result_transcript("fang.quic.rpc.result/v1", QUIC, retained, &receiver, &raw);
        Ok((transcript, (raw, receiver)))
    }

    /// Checks the envelope and signature against the retained request, then
    /// returns the individually verified records.
    pub(crate) fn verify(
        &self,
        rpc: &QuicRpc,
        retained: &[u8],
        now: u64,
    ) -> io::Result<Vec<ContactRecord>> {
        let (transcript, (raw, receiver)) = self.transcript(rpc, retained)?;
        super::verify(&self.receiver_pubkey, &transcript, &self.signature)?;
        accepted_records(raw, &rpc.request()?, &receiver, now)
    }

    /// As `verify`, for a `services` request: the whole result is rejected if
    /// any entry is malformed or names repeat.
    pub(crate) fn verify_services(
        &self,
        rpc: &QuicRpc,
        retained: &[u8],
    ) -> io::Result<Vec<PublishedService>> {
        let (transcript, (raw, _)) = self.transcript(rpc, retained)?;
        super::verify(&self.receiver_pubkey, &transcript, &self.signature)?;
        accepted_services(raw, &rpc.request()?)
    }
}

impl TcpRpc {
    pub(crate) fn request(&self) -> io::Result<RpcRequest> {
        require(self.cmd == TCP_RPC && self.protocol == TCP)?;
        RpcRequest::parse(&self.op, &self.target, self.k)
    }

    pub(crate) fn transcript(&self) -> io::Result<Vec<u8>> {
        let request = self.request()?;
        fingerprint(&self.sender_fingerprint)?;
        fingerprint(&self.receiver_fingerprint)?;
        let sp = public_matches(&self.sender_pubkey, &self.sender_fingerprint)?;
        Ok(transcript(
            "fang.tcp.rpc/v1",
            &[
                TCP.as_bytes(),
                self.sender_fingerprint.as_bytes(),
                &sp,
                self.receiver_fingerprint.as_bytes(),
                &unhex::<32>(&self.challenge)?,
                &unhex::<16>(&self.nonce)?,
                self.op.as_bytes(),
                &request.target_bytes(),
                &[request.k()],
            ],
        ))
    }

    pub(crate) fn new(
        sender: &PeltIdentity,
        challenge: &super::TcpChallenge,
        request: &RpcRequest,
    ) -> io::Result<Self> {
        challenge.validate()?;
        let (op, target, k) = request.fields();
        let mut value = Self {
            cmd: TCP_RPC.into(),
            protocol: TCP.into(),
            sender_pubkey: sender.public_key_b64.clone(),
            sender_fingerprint: sender.fingerprint.clone(),
            receiver_fingerprint: challenge.receiver_fingerprint.clone(),
            challenge: challenge.challenge.clone(),
            nonce: hex(&random::<16>()?),
            op,
            target,
            k,
            signature: String::new(),
        };
        value.signature = sign(sender, &value.transcript()?)?;
        Ok(value)
    }
}

impl TcpRpcResult {
    pub(crate) fn new<R: RpcEntry>(
        rpc: &TcpRpc,
        retained: &[u8],
        receiver: &PeltIdentity,
        records: &[R],
    ) -> io::Result<Self> {
        let mut value = Self {
            cmd: TCP_RESULT.into(),
            ok: true,
            protocol: TCP.into(),
            receiver_pubkey: receiver.public_key_b64.clone(),
            receiver_fingerprint: receiver.fingerprint.clone(),
            sender_fingerprint: rpc.sender_fingerprint.clone(),
            challenge: rpc.challenge.clone(),
            nonce: rpc.nonce.clone(),
            op: rpc.op.clone(),
            records: records
                .iter()
                .map(|r| STANDARD.encode(RpcEntry::encode(r)))
                .collect(),
            signature: String::new(),
        };
        let (transcript, _) = value.transcript(rpc, retained)?;
        value.signature = sign(receiver, &transcript)?;
        Ok(value)
    }

    fn transcript(&self, rpc: &TcpRpc, retained: &[u8]) -> io::Result<CheckedResult> {
        require(
            self.cmd == TCP_RESULT
                && self.protocol == TCP
                && self.ok
                && self.receiver_fingerprint == rpc.receiver_fingerprint
                && self.sender_fingerprint == rpc.sender_fingerprint
                && self.challenge == rpc.challenge
                && self.nonce == rpc.nonce
                && self.op == rpc.op,
        )?;
        let receiver = public_matches(&self.receiver_pubkey, &self.receiver_fingerprint)?;
        let raw = record_bytes(&self.records, rpc.request()?.max_records())?;
        let transcript =
            result_transcript("fang.tcp.rpc.result/v1", TCP, retained, &receiver, &raw);
        Ok((transcript, (raw, receiver)))
    }

    pub(crate) fn verify(
        &self,
        rpc: &TcpRpc,
        retained: &[u8],
        now: u64,
    ) -> io::Result<Vec<ContactRecord>> {
        let (transcript, (raw, receiver)) = self.transcript(rpc, retained)?;
        super::verify(&self.receiver_pubkey, &transcript, &self.signature)?;
        accepted_records(raw, &rpc.request()?, &receiver, now)
    }

    /// As `verify`, for a `services` request (see `QuicRpcResult`).
    pub(crate) fn verify_services(
        &self,
        rpc: &TcpRpc,
        retained: &[u8],
    ) -> io::Result<Vec<PublishedService>> {
        let (transcript, (raw, _)) = self.transcript(rpc, retained)?;
        super::verify(&self.receiver_pubkey, &transcript, &self.signature)?;
        accepted_services(raw, &rpc.request()?)
    }
}

/// The sender public key a TCP RPC claims, checked against its fingerprint.
pub(crate) fn tcp_sender_key(rpc: &TcpRpc) -> io::Result<[u8; 32]> {
    binary::<32>(&rpc.sender_pubkey)?;
    public_matches(&rpc.sender_pubkey, &rpc.sender_fingerprint)
}

#[cfg(test)]
mod tests;
