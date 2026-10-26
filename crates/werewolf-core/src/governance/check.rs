//! Stage30 key check (`docs/STAGE30_LIFECYCLE_SPEC.md` section 2): a lower
//! bound on the number of usable governance keys now. Nothing here talks to
//! a daemon or the network, and nothing in the network acts on a check.
//!
//! Signatures cover `KEY_CHECK_DOMAIN || network_id || challenge`, a domain
//! distinct from the statement signing domain, so a check signature is never
//! a valid statement signature and the reverse.

use super::{Head, Key, THRESHOLD};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::io;

const KEY_CHECK_DOMAIN: &[u8] = b"werewolfproxy/governance/v1/key-check\0";
const CHALLENGE_FORMAT: &str = "werewolf-key-check";
const ANSWER_FORMAT: &str = "werewolf-key-check-answer";

/// A check challenge: the network and 32 fresh random bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub format: String,
    pub version: u8,
    pub network_id: String,
    pub challenge: String,
}

/// One holder's answer to a challenge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub format: String,
    pub version: u8,
    pub network_id: String,
    pub challenge: String,
    pub index: u8,
    pub signature: String,
}

/// The only conclusions software may draw from a check. UNRECOVERABLE is a
/// legitimate-custody determination by the holders, never a verdict here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    AtLeast7Usable,
    Unverified,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::AtLeast7Usable => "at_least_7_usable",
            Verdict::Unverified => "unverified",
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Strict lowercase hex of exactly `N` bytes.
fn unhex<const N: usize>(text: &str) -> io::Result<[u8; N]> {
    let digits = text.as_bytes();
    if digits.len() != 2 * N
        || !digits
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    {
        return Err(invalid("expected lowercase hex"));
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16)
            .map_err(|_| invalid("expected lowercase hex"))?;
    }
    Ok(out)
}

fn input(network_id: &[u8; 32], challenge: &[u8; 32]) -> Vec<u8> {
    [KEY_CHECK_DOMAIN, network_id, challenge].concat()
}

impl Challenge {
    /// A fresh challenge for the network `head` belongs to.
    pub fn new(head: &Head) -> Challenge {
        let mut challenge = [0u8; 32];
        OsRng.fill_bytes(&mut challenge);
        Challenge {
            format: CHALLENGE_FORMAT.into(),
            version: 1,
            network_id: hex(&head.network_id),
            challenge: hex(&challenge),
        }
    }

    /// Strict parse of a challenge file.
    pub fn parse(bytes: &[u8]) -> io::Result<Challenge> {
        let challenge: Challenge =
            serde_json::from_slice(bytes).map_err(|_| invalid("not a key-check challenge"))?;
        challenge.fields()?;
        Ok(challenge)
    }

    fn fields(&self) -> io::Result<([u8; 32], [u8; 32])> {
        if self.format != CHALLENGE_FORMAT || self.version != 1 {
            return Err(invalid("not a key-check challenge"));
        }
        Ok((unhex(&self.network_id)?, unhex(&self.challenge)?))
    }

    /// One holder's answer, signed with `key` as governance index `index`.
    pub fn answer(&self, key: &SigningKey, index: u8) -> io::Result<Answer> {
        let (network_id, challenge) = self.fields()?;
        if index as usize >= super::KEY_COUNT {
            return Err(invalid("governance key index must be 0-9"));
        }
        let signature = key.sign(&input(&network_id, &challenge));
        Ok(Answer {
            format: ANSWER_FORMAT.into(),
            version: 1,
            network_id: self.network_id.clone(),
            challenge: self.challenge.clone(),
            index,
            signature: hex(&signature.to_bytes()),
        })
    }

    /// The indices whose answers verify against the keys in force at `head`,
    /// and the verdict. Any answer for another challenge or network, with a
    /// bad signature, or repeating an index, rejects the whole check.
    pub fn verify(&self, head: &Head, answers: &[Answer]) -> io::Result<(Vec<u8>, Verdict)> {
        let (network_id, challenge) = self.fields()?;
        if network_id != head.network_id {
            return Err(invalid("the challenge belongs to another network"));
        }
        let message = input(&network_id, &challenge);
        let mut indices = Vec::new();
        for answer in answers {
            if answer.format != ANSWER_FORMAT
                || answer.version != 1
                || answer.network_id != self.network_id
                || answer.challenge != self.challenge
            {
                return Err(invalid("an answer is for another challenge or network"));
            }
            let index = answer.index;
            if index as usize >= super::KEY_COUNT || indices.contains(&index) {
                return Err(invalid("answer indices must be distinct and 0-9"));
            }
            let key: &Key = &head.keys()[index as usize];
            let key = VerifyingKey::from_bytes(key).map_err(|_| invalid("bad governance key"))?;
            let signature = Signature::from_bytes(&unhex::<64>(&answer.signature)?);
            key.verify_strict(&message, &signature)
                .map_err(|_| invalid("an answer's signature does not verify"))?;
            indices.push(index);
        }
        indices.sort_unstable();
        let verdict = if indices.len() >= THRESHOLD {
            Verdict::AtLeast7Usable
        } else {
            Verdict::Unverified
        };
        Ok((indices, verdict))
    }
}

impl Answer {
    /// Strict parse of an answer file.
    pub fn parse(bytes: &[u8]) -> io::Result<Answer> {
        let answer: Answer =
            serde_json::from_slice(bytes).map_err(|_| invalid("not a key-check answer"))?;
        if answer.format != ANSWER_FORMAT || answer.version != 1 {
            return Err(invalid("not a key-check answer"));
        }
        unhex::<32>(&answer.network_id)?;
        unhex::<32>(&answer.challenge)?;
        unhex::<64>(&answer.signature)?;
        Ok(answer)
    }
}

/// Writes a new challenge or answer file; never replaces an existing one.
pub fn write<T: Serialize>(path: &std::path::Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| invalid("cannot encode"))?;
    super::tool::create(path, &bytes, 0o644)
}

/// Answers the challenge file at `challenge` with the governance key file at
/// `key` (the same key-file checks as `governance sign`).
pub fn sign_file(
    challenge: &std::path::Path,
    key: &std::path::Path,
    index: u8,
) -> io::Result<Answer> {
    let challenge = Challenge::parse(&std::fs::read(challenge)?)?;
    challenge.answer(&super::tool::read_key(key)?, index)
}

#[cfg(test)]
pub(crate) fn input_for_tests(network_id: &[u8; 32], challenge: &[u8; 32]) -> Vec<u8> {
    input(network_id, challenge)
}
