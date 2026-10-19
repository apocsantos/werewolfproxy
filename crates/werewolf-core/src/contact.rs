//! Signed contact records for Stage24 peer discovery
//! (`docs/STAGE24_FINDNODE_DESIGN.md`).
//!
//! A record says where a node can be reached. The node signs its own record,
//! so a peer relaying it cannot forge or alter it; a relay can still withhold
//! records or serve stale ones, which `seq` ordering limits. A record never
//! grants trust: adding a discovered node to the Pack stays an operator
//! action.

use crate::nodeid::NodeId;
use crate::pelt::{sign_message, PeltIdentity};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, VerifyingKey};
use std::net::SocketAddr;

/// Signature domain; never reused for any other signed message.
pub const DOMAIN: &[u8] = b"werewolf/contact/v1\0";
pub const MAX_ADDRESSES: usize = 4;
pub const MAX_ADDRESS_LEN: usize = 256;
/// A record may be valid for at most seven days from the verifier's clock.
pub const MAX_LIFETIME_SECS: u64 = 7 * 24 * 60 * 60;
/// Upper bound of an encoded record, checked before any parsing.
pub const MAX_ENCODED_LEN: usize = 32 + 1 + MAX_ADDRESSES * (2 + MAX_ADDRESS_LEN) + 8 + 8 + 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactRecord {
    pub public_key: [u8; 32],
    pub addresses: Vec<String>,
    pub seq: u64,
    pub expires_at: u64,
    pub signature: [u8; 64],
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContactError {
    #[error("contact record must list 1 to {MAX_ADDRESSES} addresses")]
    AddressCount,
    #[error("contact address must be quic:// or tcp:// followed by a literal IP:port")]
    Address,
    #[error("contact record is expired or valid for too long")]
    Lifetime,
    #[error("contact record public key is invalid")]
    PublicKey,
    #[error("contact record signature is invalid")]
    Signature,
    #[error("contact record encoding is malformed")]
    Malformed,
    #[error("contact record could not be signed")]
    Signing,
}

impl ContactRecord {
    /// Signs a record for `identity`. `now` and `expires_at` are Unix seconds.
    pub fn sign(
        identity: &PeltIdentity,
        addresses: Vec<String>,
        seq: u64,
        expires_at: u64,
        now: u64,
    ) -> Result<Self, ContactError> {
        let public_key: [u8; 32] = STANDARD
            .decode(&identity.public_key_b64)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(ContactError::PublicKey)?;
        let mut record = Self {
            public_key,
            addresses,
            seq,
            expires_at,
            signature: [0; 64],
        };
        record.check_fields(now)?;
        let signature =
            sign_message(identity, &record.signed_bytes()).map_err(|_| ContactError::Signing)?;
        record.signature = STANDARD
            .decode(signature)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(ContactError::Signing)?;
        Ok(record)
    }

    /// Checks fields, lifetime against `now` and the strict Ed25519
    /// signature. Returns the node ID the record speaks for.
    pub fn verify(&self, now: u64) -> Result<NodeId, ContactError> {
        self.check_fields(now)?;
        let key =
            VerifyingKey::from_bytes(&self.public_key).map_err(|_| ContactError::PublicKey)?;
        key.verify_strict(
            &self.signed_bytes(),
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|_| ContactError::Signature)?;
        Ok(self.node_id())
    }

    pub fn node_id(&self) -> NodeId {
        NodeId::from_public_key(&self.public_key)
    }

    /// True if `self` should replace `older` for the same node. Both
    /// records must already be verified.
    pub fn supersedes(&self, older: &Self) -> bool {
        self.public_key == older.public_key && self.seq > older.seq
    }

    /// Canonical encoding: public key, address count, length-prefixed
    /// addresses, seq, expires_at (big-endian), signature.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.body();
        out.extend_from_slice(&self.signature);
        out
    }

    /// Strict inverse of `encode`: bounded, exact length, no trailing bytes.
    /// Decoding does not verify; call `verify` before using the record.
    pub fn decode(bytes: &[u8]) -> Result<Self, ContactError> {
        if bytes.len() > MAX_ENCODED_LEN {
            return Err(ContactError::Malformed);
        }
        let mut input = Reader(bytes);
        let public_key = input.array::<32>()?;
        let count = input.array::<1>()?[0] as usize;
        if count == 0 || count > MAX_ADDRESSES {
            return Err(ContactError::AddressCount);
        }
        let mut addresses = Vec::with_capacity(count);
        for _ in 0..count {
            let len = u16::from_be_bytes(input.array::<2>()?) as usize;
            if len > MAX_ADDRESS_LEN {
                return Err(ContactError::Address);
            }
            let text = std::str::from_utf8(input.take(len)?).map_err(|_| ContactError::Address)?;
            addresses.push(text.to_string());
        }
        let seq = u64::from_be_bytes(input.array::<8>()?);
        let expires_at = u64::from_be_bytes(input.array::<8>()?);
        let signature = input.array::<64>()?;
        if !input.0.is_empty() {
            return Err(ContactError::Malformed);
        }
        Ok(Self {
            public_key,
            addresses,
            seq,
            expires_at,
            signature,
        })
    }

    fn check_fields(&self, now: u64) -> Result<(), ContactError> {
        if self.addresses.is_empty() || self.addresses.len() > MAX_ADDRESSES {
            return Err(ContactError::AddressCount);
        }
        if !self.addresses.iter().all(|address| valid_address(address)) {
            return Err(ContactError::Address);
        }
        if self.expires_at <= now || self.expires_at - now > MAX_LIFETIME_SECS {
            return Err(ContactError::Lifetime);
        }
        Ok(())
    }

    fn body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAX_ENCODED_LEN);
        out.extend_from_slice(&self.public_key);
        out.push(self.addresses.len() as u8);
        for address in &self.addresses {
            out.extend_from_slice(&(address.len() as u16).to_be_bytes());
            out.extend_from_slice(address.as_bytes());
        }
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        out
    }

    fn signed_bytes(&self) -> Vec<u8> {
        let mut out = DOMAIN.to_vec();
        out.extend_from_slice(&self.body());
        out
    }
}

/// `quic://` or `tcp://` followed by a literal, specified IP and a non-zero
/// port. Hostnames are rejected: discovery never triggers DNS lookups.
fn valid_address(address: &str) -> bool {
    if address.len() > MAX_ADDRESS_LEN {
        return false;
    }
    let Some(endpoint) = address
        .strip_prefix("quic://")
        .or_else(|| address.strip_prefix("tcp://"))
    else {
        return false;
    };
    endpoint
        .parse::<SocketAddr>()
        .is_ok_and(|socket| socket.port() != 0 && !socket.ip().is_unspecified())
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], ContactError> {
        if self.0.len() < len {
            return Err(ContactError::Malformed);
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ContactError> {
        Ok(self.take(N)?.try_into().expect("take returned N bytes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pelt::generate_identity;

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 24 * 60 * 60;

    fn record(identity: &PeltIdentity, seq: u64) -> ContactRecord {
        ContactRecord::sign(
            identity,
            vec![
                "quic://203.0.113.7:9560".into(),
                "tcp://[2001:db8::1]:8443".into(),
            ],
            seq,
            NOW + DAY,
            NOW,
        )
        .unwrap()
    }

    #[test]
    fn signed_record_verifies_to_the_pelt_node_id() {
        let identity = generate_identity();
        let signed = record(&identity, 1);
        let key: [u8; 32] = STANDARD
            .decode(&identity.public_key_b64)
            .unwrap()
            .try_into()
            .unwrap();

        assert_eq!(signed.verify(NOW), Ok(NodeId::from_public_key(&key)));
    }

    #[test]
    fn encoding_round_trips_exactly() {
        let signed = record(&generate_identity(), 7);
        let bytes = signed.encode();

        assert_eq!(ContactRecord::decode(&bytes), Ok(signed.clone()));
        assert!(bytes.len() <= MAX_ENCODED_LEN);
        assert_eq!(
            ContactRecord::decode(&bytes).unwrap().verify(NOW),
            signed.verify(NOW)
        );
    }

    #[test]
    fn any_tampered_field_fails_verification() {
        let signed = record(&generate_identity(), 1);
        let mut address = signed.clone();
        address.addresses[0] = "quic://198.51.100.9:9560".into();
        let mut seq = signed.clone();
        seq.seq += 1;
        let mut expiry = signed.clone();
        expiry.expires_at += 1;
        let mut key = signed.clone();
        key.public_key = record(&generate_identity(), 1).public_key;

        for tampered in [address, seq, expiry, key] {
            assert_eq!(tampered.verify(NOW), Err(ContactError::Signature));
        }
    }

    #[test]
    fn signature_is_domain_separated() {
        // A plain Pelt signature over the same body must not verify as a record.
        let identity = generate_identity();
        let mut signed = record(&identity, 1);
        let plain = sign_message(&identity, &signed.body()).unwrap();
        signed.signature = STANDARD.decode(plain).unwrap().try_into().unwrap();

        assert_eq!(signed.verify(NOW), Err(ContactError::Signature));
    }

    #[test]
    fn lifetime_is_bounded_on_both_sides() {
        let identity = generate_identity;
        let addresses = || vec!["quic://203.0.113.7:9560".to_string()];

        let at_limit =
            ContactRecord::sign(&identity(), addresses(), 1, NOW + MAX_LIFETIME_SECS, NOW).unwrap();
        assert!(at_limit.verify(NOW).is_ok());
        assert_eq!(
            at_limit.verify(NOW + MAX_LIFETIME_SECS),
            Err(ContactError::Lifetime)
        );
        assert_eq!(
            ContactRecord::sign(
                &identity(),
                addresses(),
                1,
                NOW + MAX_LIFETIME_SECS + 1,
                NOW
            ),
            Err(ContactError::Lifetime)
        );
        // Too far in the future from the verifier's clock, e.g. a skewed signer.
        assert_eq!(at_limit.verify(NOW - 1), Err(ContactError::Lifetime));
    }

    #[test]
    fn addresses_must_be_literal_specified_endpoints() {
        let identity = generate_identity();
        for bad in [
            vec![],
            vec!["quic://example.org:9560".to_string()],
            vec!["http://203.0.113.7:80".to_string()],
            vec!["quic://203.0.113.7:0".to_string()],
            vec!["tcp://0.0.0.0:8443".to_string()],
            vec!["quic://203.0.113.7:9560 ".to_string()],
            vec!["quic://203.0.113.7:9560".to_string(); MAX_ADDRESSES + 1],
        ] {
            assert!(ContactRecord::sign(&identity, bad, 1, NOW + DAY, NOW).is_err());
        }
    }

    #[test]
    fn decode_rejects_truncated_padded_and_oversized_input() {
        let bytes = record(&generate_identity(), 1).encode();

        for len in 0..bytes.len() {
            assert!(ContactRecord::decode(&bytes[..len]).is_err());
        }
        let mut padded = bytes.clone();
        padded.push(0);
        assert_eq!(ContactRecord::decode(&padded), Err(ContactError::Malformed));
        assert_eq!(
            ContactRecord::decode(&vec![0; MAX_ENCODED_LEN + 1]),
            Err(ContactError::Malformed)
        );

        let mut zero_addresses = bytes.clone();
        zero_addresses[32] = 0;
        assert_eq!(
            ContactRecord::decode(&zero_addresses),
            Err(ContactError::AddressCount)
        );
    }

    #[test]
    fn higher_seq_supersedes_only_for_the_same_node() {
        let identity = generate_identity();
        let old = record(&identity, 1);
        let new = record(&identity, 2);
        let other = record(&generate_identity(), 9);

        assert!(new.supersedes(&old));
        assert!(!old.supersedes(&new));
        assert!(!old.supersedes(&old));
        assert!(!other.supersedes(&old));
    }
}
