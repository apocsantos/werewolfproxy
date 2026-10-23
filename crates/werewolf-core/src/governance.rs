//! Stage28 governance statements
//! (`docs/STAGE28_GOVERNANCE_STATEMENT_SPEC.md`).
//!
//! A statement is one governance action signed by at least 7 of the 10
//! governance keys. This module encodes, decodes and verifies statements
//! and applies them to a chain head; it holds no private key except in the
//! offline `sign` helper, and it touches no Pack or file.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

pub const VERSION: u8 = 1;
pub const MAX_STATEMENT_LEN: usize = 16 * 1024;
pub const KEY_COUNT: usize = 10;
pub const THRESHOLD: usize = 7;
pub const MAX_REVOKED_PER_STATEMENT: usize = 64;
/// Stage29 addition C.
pub const MAX_ROSTER: usize = 1024;
pub const MAX_ADMITTED_PER_STATEMENT: usize = 64;

const SIGN_DOMAIN: &[u8] = b"werewolfproxy/governance/v1/sign\0";
const HASH_DOMAIN: &[u8] = b"werewolfproxy/governance/v1/hash\0";

const KIND_GENESIS: u8 = 0;
const KIND_ROTATION: u8 = 1;
const KIND_REVOCATION: u8 = 2;
const KIND_ADMISSION: u8 = 3;
const KIND_ACTIVATION: u8 = 4;

pub type Key = [u8; 32];
pub type Hash = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Genesis {
        threshold: u8,
        keys: Vec<Key>,
    },
    /// `(index, new key)`, indices strictly increasing.
    Rotation(Vec<(u8, Key)>),
    /// Member public keys, strictly increasing byte order.
    Revocation(Vec<Key>),
    /// Stage29: member public keys admitted to the roster, strictly
    /// increasing byte order.
    Admission(Vec<Key>),
    /// Stage29: the network is active from this statement on.
    Activation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Core {
    pub network_id: [u8; 32],
    pub sequence: u64,
    pub previous: Hash,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub core: Core,
    /// `(key index, signature)`, indices strictly increasing.
    pub signatures: Vec<(u8, [u8; 64])>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum GovError {
    #[error("statement exceeds {MAX_STATEMENT_LEN} bytes")]
    TooLarge,
    #[error("statement encoding is malformed")]
    Malformed,
    #[error("unknown statement version")]
    UnknownVersion,
    #[error("unknown statement kind")]
    UnknownKind,
    #[error("statement body violates its rules")]
    BodyRule,
    #[error("not enough valid, distinct, ordered signatures")]
    Signatures,
    #[error("statement belongs to another network")]
    WrongNetwork,
    #[error("no genesis applied yet")]
    NoGenesis,
    #[error("genesis does not match the expected hash")]
    GenesisMismatch,
    #[error("statement is not the next in the chain")]
    OutOfOrder,
    #[error("statement does not follow the current head")]
    PreviousMismatch,
    #[error("a validly signed statement conflicts with an applied one")]
    Fork,
    #[error("governance is fenced after a fork")]
    Forked,
}

impl Body {
    fn kind(&self) -> u8 {
        match self {
            Body::Genesis { .. } => KIND_GENESIS,
            Body::Rotation(_) => KIND_ROTATION,
            Body::Revocation(_) => KIND_REVOCATION,
            Body::Admission(_) => KIND_ADMISSION,
            Body::Activation => KIND_ACTIVATION,
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Body::Genesis { threshold, keys } => {
                out.push(*threshold);
                out.push(keys.len() as u8);
                keys.iter().for_each(|key| out.extend_from_slice(key));
            }
            Body::Rotation(entries) => {
                out.push(entries.len() as u8);
                for (index, key) in entries {
                    out.push(*index);
                    out.extend_from_slice(key);
                }
            }
            Body::Revocation(keys) | Body::Admission(keys) => {
                out.extend_from_slice(&(keys.len() as u16).to_be_bytes());
                keys.iter().for_each(|key| out.extend_from_slice(key));
            }
            Body::Activation => {}
        }
        out
    }

    fn decode(kind: u8, bytes: &[u8]) -> Result<Self, GovError> {
        let mut r = Reader(bytes);
        let body = match kind {
            KIND_GENESIS => {
                let threshold = r.u8()?;
                let count = r.u8()? as usize;
                let keys = (0..count).map(|_| r.array()).collect::<Result<_, _>>()?;
                Body::Genesis { threshold, keys }
            }
            KIND_ROTATION => {
                let count = r.u8()? as usize;
                let entries = (0..count)
                    .map(|_| Ok((r.u8()?, r.array()?)))
                    .collect::<Result<_, GovError>>()?;
                Body::Rotation(entries)
            }
            KIND_REVOCATION | KIND_ADMISSION => {
                let count = r.u16()? as usize;
                let keys = (0..count).map(|_| r.array()).collect::<Result<_, _>>()?;
                if kind == KIND_REVOCATION {
                    Body::Revocation(keys)
                } else {
                    Body::Admission(keys)
                }
            }
            KIND_ACTIVATION => Body::Activation,
            _ => return Err(GovError::UnknownKind),
        };
        r.end()?;
        Ok(body)
    }
}

impl Core {
    /// The canonical core bytes (spec "Encoding"); the signatures are not
    /// part of it.
    pub fn encode(&self) -> Vec<u8> {
        let body = self.body.encode();
        let mut out = Vec::with_capacity(76 + body.len());
        out.push(VERSION);
        out.extend_from_slice(&self.network_id);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.previous);
        out.push(self.body.kind());
        out.extend_from_slice(&(body.len() as u16).to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    pub fn signing_input(&self) -> Vec<u8> {
        [SIGN_DOMAIN, &self.encode()].concat()
    }

    pub fn hash(&self) -> Hash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(HASH_DOMAIN);
        hasher.update(&self.encode());
        *hasher.finalize().as_bytes()
    }
}

/// Offline signing by one key holder (survey decision 2a). The daemon never
/// calls this.
pub fn sign(core: &Core, key: &SigningKey) -> [u8; 64] {
    key.sign(&core.signing_input()).to_bytes()
}

impl Statement {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.core.encode();
        out.push(self.signatures.len() as u8);
        for (index, signature) in &self.signatures {
            out.push(*index);
            out.extend_from_slice(signature);
        }
        out
    }

    /// Strict decoding of the encoding only; rules are checked by `apply`.
    pub fn decode(bytes: &[u8]) -> Result<Self, GovError> {
        if bytes.len() > MAX_STATEMENT_LEN {
            return Err(GovError::TooLarge);
        }
        let mut r = Reader(bytes);
        if r.u8()? != VERSION {
            return Err(GovError::UnknownVersion);
        }
        let network_id = r.array()?;
        let sequence = r.u64()?;
        let previous = r.array()?;
        let kind = r.u8()?;
        let body_len = r.u16()? as usize;
        let body = Body::decode(kind, r.take(body_len)?)?;
        let count = r.u8()? as usize;
        let signatures = (0..count)
            .map(|_| Ok((r.u8()?, r.array()?)))
            .collect::<Result<_, GovError>>()?;
        r.end()?;
        Ok(Self {
            core: Core {
                network_id,
                sequence,
                previous,
                body,
            },
            signatures,
        })
    }
}

/// What a node knows of its network's governance (spec "Persistence").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub network_id: [u8; 32],
    pub sequence: u64,
    pub hash: Hash,
    pub threshold: u8,
    /// `(sequence that set them, keys)`: the keys in force for a statement
    /// with sequence `s` are those of the last entry set before `s`.
    pub key_epochs: Vec<(u64, Vec<Key>)>,
    /// Network-revoked member keys, sorted.
    pub revoked: Vec<Key>,
    /// The hash of every applied statement, indexed by sequence.
    pub applied: Vec<Hash>,
    pub forked: bool,
    /// Stage29: admitted member keys, sorted, never revoked ones.
    pub roster: Vec<Key>,
    /// Stage29: the sequence of the activation statement, once applied.
    pub activated_at: Option<u64>,
}

impl Head {
    /// Stage29 local admission (addendum "Local admission"): activation is a
    /// network-chain fact; whether `own` may act as a member is derived from
    /// it. Before activation every node is eligible, as in Stage28.
    pub fn admits(&self, own: &Key) -> bool {
        self.activated_at.is_none()
            || (self.roster.binary_search(own).is_ok() && self.revoked.binary_search(own).is_err())
    }

    pub fn keys(&self) -> &[Key] {
        &self.key_epochs.last().expect("genesis sets keys").1
    }

    fn keys_before(&self, sequence: u64) -> &[Key] {
        &self
            .key_epochs
            .iter()
            .rev()
            .find(|(set_at, _)| *set_at < sequence)
            .expect("genesis precedes every later sequence")
            .1
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The new head; persist it before acting on it.
    Applied(Head),
    AlreadyApplied,
    /// A conflicting, validly signed statement: the returned head is the
    /// current one with `forked` set; persist it.
    Fork(Head),
}

/// Verifies `statement` against `head` (spec "Verification") and returns
/// what applying it means. `expect_genesis` pins the genesis hash (S1).
pub fn apply(
    head: Option<&Head>,
    statement: &Statement,
    expect_genesis: Option<&Hash>,
) -> Result<Outcome, GovError> {
    let core = &statement.core;
    let Some(head) = head else {
        let Body::Genesis { threshold, keys } = &core.body else {
            return Err(GovError::NoGenesis);
        };
        if core.sequence != 0 || core.previous != [0; 32] {
            return Err(GovError::OutOfOrder);
        }
        if expect_genesis != Some(&core.hash()) {
            return Err(GovError::GenesisMismatch);
        }
        check_genesis(*threshold, keys)?;
        check_signatures(statement, keys, *threshold as usize)?;
        return Ok(Outcome::Applied(Head {
            network_id: core.network_id,
            sequence: 0,
            hash: core.hash(),
            threshold: *threshold,
            key_epochs: vec![(0, keys.clone())],
            revoked: Vec::new(),
            applied: vec![core.hash()],
            forked: false,
            roster: Vec::new(),
            activated_at: None,
        }));
    };
    if head.forked {
        return Err(GovError::Forked);
    }
    if core.network_id != head.network_id {
        return Err(GovError::WrongNetwork);
    }
    if core.sequence <= head.sequence {
        let recorded = head.applied[core.sequence as usize];
        if recorded == core.hash() {
            return Ok(Outcome::AlreadyApplied);
        }
        // A different genesis is not evidence about this network's keys:
        // the genesis is pinned, so it is simply refused.
        if core.sequence == 0 {
            return Err(GovError::GenesisMismatch);
        }
        check_signatures(
            statement,
            head.keys_before(core.sequence),
            head.threshold as usize,
        )?;
        let mut fenced = head.clone();
        fenced.forked = true;
        return Ok(Outcome::Fork(fenced));
    }
    if core.sequence != head.sequence + 1 {
        return Err(GovError::OutOfOrder);
    }
    if core.previous != head.hash {
        return Err(GovError::PreviousMismatch);
    }
    check_signatures(statement, head.keys(), head.threshold as usize)?;
    let mut next = head.clone();
    match &core.body {
        Body::Genesis { .. } => return Err(GovError::OutOfOrder),
        Body::Rotation(entries) => {
            let keys = rotated(head.keys(), entries)?;
            next.key_epochs.push((core.sequence, keys));
        }
        Body::Revocation(keys) => {
            check_revocation(keys)?;
            for key in keys {
                if let Err(at) = next.revoked.binary_search(key) {
                    next.revoked.insert(at, *key);
                }
                // A revoked key leaves the roster (Stage29).
                if let Ok(at) = next.roster.binary_search(key) {
                    next.roster.remove(at);
                }
            }
        }
        Body::Admission(keys) => {
            check_admission(head, keys)?;
            for key in keys {
                let at = next
                    .roster
                    .binary_search(key)
                    .expect_err("checked not admitted");
                next.roster.insert(at, *key);
            }
        }
        Body::Activation => {
            // A network-chain fact, applied on every node (addition A).
            if head.activated_at.is_some() {
                return Err(GovError::BodyRule);
            }
            next.activated_at = Some(core.sequence);
        }
    }
    next.sequence = core.sequence;
    next.hash = core.hash();
    next.applied.push(next.hash);
    Ok(Outcome::Applied(next))
}

fn check_genesis(threshold: u8, keys: &[Key]) -> Result<(), GovError> {
    if threshold as usize != THRESHOLD || keys.len() != KEY_COUNT {
        return Err(GovError::BodyRule);
    }
    for (index, key) in keys.iter().enumerate() {
        strong_key(key)?;
        if keys[..index].contains(key) {
            return Err(GovError::BodyRule);
        }
    }
    Ok(())
}

fn rotated(current: &[Key], entries: &[(u8, Key)]) -> Result<Vec<Key>, GovError> {
    if entries.is_empty() || entries.len() > KEY_COUNT {
        return Err(GovError::BodyRule);
    }
    let mut keys = current.to_vec();
    let mut last = None;
    for (index, key) in entries {
        if last.is_some_and(|last| last >= *index) || *index as usize >= KEY_COUNT {
            return Err(GovError::BodyRule);
        }
        last = Some(*index);
        strong_key(key)?;
        keys[*index as usize] = *key;
    }
    // Every key in the resulting set distinct; a rotation never brings back
    // a key it replaces or duplicates one it keeps.
    for (index, key) in keys.iter().enumerate() {
        if keys[..index].contains(key) {
            return Err(GovError::BodyRule);
        }
    }
    for (index, key) in entries {
        if current[*index as usize] == *key {
            return Err(GovError::BodyRule);
        }
    }
    Ok(keys)
}

/// Stage29 admission: 1–64 sorted strong keys, none revoked (permanently:
/// addition B), none already admitted, and the roster stays within 1024.
fn check_admission(head: &Head, keys: &[Key]) -> Result<(), GovError> {
    if keys.is_empty() || keys.len() > MAX_ADMITTED_PER_STATEMENT {
        return Err(GovError::BodyRule);
    }
    if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(GovError::BodyRule);
    }
    for key in keys {
        strong_key(key)?;
        if head.revoked.binary_search(key).is_ok() || head.roster.binary_search(key).is_ok() {
            return Err(GovError::BodyRule);
        }
    }
    if head.roster.len() + keys.len() > MAX_ROSTER {
        return Err(GovError::BodyRule);
    }
    Ok(())
}

fn check_revocation(keys: &[Key]) -> Result<(), GovError> {
    if keys.is_empty() || keys.len() > MAX_REVOKED_PER_STATEMENT {
        return Err(GovError::BodyRule);
    }
    if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(GovError::BodyRule);
    }
    for key in keys {
        VerifyingKey::from_bytes(key).map_err(|_| GovError::BodyRule)?;
    }
    Ok(())
}

/// A valid Ed25519 point that is not of small order.
fn strong_key(key: &Key) -> Result<VerifyingKey, GovError> {
    let key = VerifyingKey::from_bytes(key).map_err(|_| GovError::BodyRule)?;
    if key.is_weak() {
        return Err(GovError::BodyRule);
    }
    Ok(key)
}

fn check_signatures(statement: &Statement, keys: &[Key], threshold: usize) -> Result<(), GovError> {
    let signatures = &statement.signatures;
    if signatures.len() < threshold || signatures.len() > keys.len() {
        return Err(GovError::Signatures);
    }
    let input = statement.core.signing_input();
    let mut last = None;
    for (index, signature) in signatures {
        if last.is_some_and(|last| last >= *index) || *index as usize >= keys.len() {
            return Err(GovError::Signatures);
        }
        last = Some(*index);
        let key = strong_key(&keys[*index as usize]).map_err(|_| GovError::Signatures)?;
        key.verify_strict(&input, &Signature::from_bytes(signature))
            .map_err(|_| GovError::Signatures)?;
    }
    Ok(())
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], GovError> {
        if self.0.len() < len {
            return Err(GovError::Malformed);
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, GovError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, GovError> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, GovError> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], GovError> {
        Ok(self.take(N)?.try_into().expect("length checked"))
    }
    fn end(&self) -> Result<(), GovError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(GovError::Malformed)
        }
    }
}

pub mod tool;

#[cfg(test)]
mod tests;
