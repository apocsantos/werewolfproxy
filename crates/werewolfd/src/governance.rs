//! Stage28 governance on the node (`docs/STAGE28_GOVERNANCE_STATEMENT_SPEC.md`):
//! the persisted form of the chain head, a protected manifest document.

use crate::state::DaemonState;
use serde::{Deserialize, Serialize};
use std::io;
use werewolf_core::{
    governance::{tool, Head, Key, KEY_COUNT, THRESHOLD},
    pack::PeerRecord,
};

/// This node's own Pelt public key, if it has a Pelt.
pub(crate) fn own_key(st: &DaemonState) -> Option<Key> {
    st.pelt
        .as_ref()
        .and_then(|pelt| tool::public_key(&pelt.public_key_b64).ok())
}

/// Stage29 local admission: activation is a network-chain fact; whether this
/// node may act as a member is derived from the head and its current Pelt,
/// never stored. A node without a Pelt is unadmitted once activated.
pub(crate) fn unadmitted(st: &DaemonState) -> bool {
    match (&st.governance, own_key(st)) {
        (Some(head), Some(own)) => !head.admits(&own),
        (Some(head), None) => head.activated_at.is_some(),
        (None, _) => false,
    }
}

/// Whether the head excludes this Pack member: its key is network-revoked,
/// or, after activation, it is not on the roster (or has no full key to
/// check against the roster).
pub(crate) fn excludes(head: &Head, peer: &PeerRecord) -> bool {
    match peer
        .public_key_b64
        .as_deref()
        .and_then(|b64| tool::public_key(b64).ok())
    {
        Some(key) => {
            head.revoked.binary_search(&key).is_ok()
                || (head.activated_at.is_some() && head.roster.binary_search(&key).is_err())
        }
        None => head.activated_at.is_some(),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    network_id: String,
    sequence: u64,
    hash: String,
    threshold: u8,
    key_epochs: Vec<KeyEpoch>,
    revoked: Vec<String>,
    applied: Vec<String>,
    forked: bool,
    /// Stage29 fields, omitted while empty so a Stage28 head keeps its bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    roster: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activated_at: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyEpoch {
    set_at: u64,
    keys: Vec<String>,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid governance document")
}

fn document(head: &Head) -> Document {
    Document {
        version: 1,
        network_id: tool::hex(&head.network_id),
        sequence: head.sequence,
        hash: tool::hex(&head.hash),
        threshold: head.threshold,
        key_epochs: head
            .key_epochs
            .iter()
            .map(|(set_at, keys)| KeyEpoch {
                set_at: *set_at,
                keys: keys.iter().map(|k| tool::hex(k)).collect(),
            })
            .collect(),
        revoked: head.revoked.iter().map(|k| tool::hex(k)).collect(),
        applied: head.applied.iter().map(|h| tool::hex(h)).collect(),
        forked: head.forked,
        roster: head.roster.iter().map(|k| tool::hex(k)).collect(),
        activated_at: head.activated_at,
    }
}

/// The canonical bytes of `head`, after checking it is well formed.
pub(crate) fn canonical(head: &Head) -> io::Result<Vec<u8>> {
    check(head)?;
    serde_json::to_vec(&document(head)).map_err(|_| invalid())
}

/// Strict parse: exactly the canonical bytes of a well-formed head.
pub(crate) fn parse(bytes: &[u8]) -> io::Result<Head> {
    let doc: Document = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if doc.version != 1 {
        return Err(invalid());
    }
    let hex32 = |text: &String| tool::unhex::<32>(text).map_err(|_| invalid());
    let head = Head {
        network_id: hex32(&doc.network_id)?,
        sequence: doc.sequence,
        hash: hex32(&doc.hash)?,
        threshold: doc.threshold,
        key_epochs: doc
            .key_epochs
            .iter()
            .map(|epoch| {
                Ok((
                    epoch.set_at,
                    epoch
                        .keys
                        .iter()
                        .map(hex32)
                        .collect::<io::Result<Vec<Key>>>()?,
                ))
            })
            .collect::<io::Result<_>>()?,
        revoked: doc.revoked.iter().map(hex32).collect::<io::Result<_>>()?,
        applied: doc.applied.iter().map(hex32).collect::<io::Result<_>>()?,
        forked: doc.forked,
        roster: doc.roster.iter().map(hex32).collect::<io::Result<_>>()?,
        activated_at: doc.activated_at,
    };
    if canonical(&head)? != bytes {
        return Err(invalid());
    }
    Ok(head)
}

/// The structural invariants `werewolf_core::governance::apply` maintains.
fn check(head: &Head) -> io::Result<()> {
    let ok = head.threshold as usize == THRESHOLD
        && head.applied.len() as u64 == head.sequence.wrapping_add(1)
        && head.applied.last() == Some(&head.hash)
        && head
            .key_epochs
            .first()
            .is_some_and(|(set_at, _)| *set_at == 0)
        && head.key_epochs.windows(2).all(|pair| pair[0].0 < pair[1].0)
        && head
            .key_epochs
            .last()
            .is_some_and(|(set_at, _)| *set_at <= head.sequence)
        && head.key_epochs.iter().all(|(_, keys)| {
            keys.len() == KEY_COUNT
                && keys
                    .iter()
                    .enumerate()
                    .all(|(index, key)| !keys[..index].contains(key))
        })
        && head.revoked.windows(2).all(|pair| pair[0] < pair[1])
        && head.roster.len() <= werewolf_core::governance::MAX_ROSTER
        && head.roster.windows(2).all(|pair| pair[0] < pair[1])
        && head
            .roster
            .iter()
            .all(|key| head.revoked.binary_search(key).is_err())
        && head.activated_at.is_none_or(|at| at <= head.sequence);
    if ok {
        Ok(())
    } else {
        Err(invalid())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use werewolf_core::governance::{apply, sign, Body, Core, Outcome, Statement};

    pub(crate) fn sample_head() -> Head {
        let keys: Vec<Key> = (0..10u8)
            .map(|i| {
                SigningKey::from_bytes(&[i + 1; 32])
                    .verifying_key()
                    .to_bytes()
            })
            .collect();
        let core = Core {
            network_id: [0x77; 32],
            sequence: 0,
            previous: [0; 32],
            body: Body::Genesis { threshold: 7, keys },
        };
        let signatures = (0..7u8)
            .map(|i| (i, sign(&core, &SigningKey::from_bytes(&[i + 1; 32]))))
            .collect();
        let genesis = Statement { core, signatures };
        match apply(None, &genesis, Some(&genesis.core.hash())).unwrap() {
            Outcome::Applied(head) => head,
            other => panic!("{other:?}"),
        }
    }

    /// `head` after one more statement: revoking a fixed member key.
    pub(crate) fn revoked_head(head: &Head) -> Head {
        let member = SigningKey::from_bytes(&[0xA1; 32])
            .verifying_key()
            .to_bytes();
        let core = Core {
            network_id: head.network_id,
            sequence: head.sequence + 1,
            previous: head.hash,
            body: Body::Revocation(vec![member]),
        };
        let signatures = (0..7u8)
            .map(|i| (i, sign(&core, &SigningKey::from_bytes(&[i + 1; 32]))))
            .collect();
        match apply(Some(head), &Statement { core, signatures }, None).unwrap() {
            Outcome::Applied(next) => next,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn round_trips_canonically_and_rejects_bad_heads() {
        let head = sample_head();
        let bytes = canonical(&head).unwrap();
        assert_eq!(parse(&bytes).unwrap(), head);
        // Not canonical: whitespace.
        let spaced = String::from_utf8(bytes.clone()).unwrap().replace(':', ": ");
        assert!(parse(spaced.as_bytes()).is_err());
        // Structural invariants.
        let broken = [
            Head {
                threshold: 6,
                ..head.clone()
            },
            Head {
                sequence: 1,
                ..head.clone()
            },
            Head {
                hash: [1; 32],
                ..head.clone()
            },
            Head {
                key_epochs: vec![],
                ..head.clone()
            },
            Head {
                revoked: vec![[2; 32], [1; 32]],
                ..head.clone()
            },
            {
                let mut h = head.clone();
                h.key_epochs[0].1[9] = h.key_epochs[0].1[0];
                h
            },
        ];
        for head in broken {
            assert!(canonical(&head).is_err());
        }
        assert!(parse(b"{}").is_err());
    }

    #[test]
    fn stage28_heads_keep_their_bytes_and_stage29_fields_are_checked() {
        let head = sample_head();
        let stage28 = canonical(&head).unwrap();
        let text = String::from_utf8(stage28.clone()).unwrap();
        assert!(
            !text.contains("roster") && !text.contains("activated_at"),
            "{text}"
        );
        assert_eq!(parse(&stage28).unwrap(), head);

        let a = SigningKey::from_bytes(&[0xB1; 32])
            .verifying_key()
            .to_bytes();
        let b = SigningKey::from_bytes(&[0xB2; 32])
            .verifying_key()
            .to_bytes();
        let mut roster = vec![a, b];
        roster.sort();
        let stage29 = Head {
            roster: roster.clone(),
            activated_at: Some(0),
            ..head.clone()
        };
        let bytes = canonical(&stage29).unwrap();
        assert_eq!(parse(&bytes).unwrap(), stage29);
        for broken in [
            Head {
                roster: vec![roster[1], roster[0]],
                ..stage29.clone()
            },
            Head {
                roster: vec![a, a],
                ..stage29.clone()
            },
            Head {
                revoked: vec![a],
                roster: vec![a],
                ..stage29.clone()
            },
            Head {
                activated_at: Some(1),
                ..stage29.clone()
            },
            Head {
                roster: (0..1025u32)
                    .map(|i| {
                        let mut key = [0u8; 32];
                        key[..4].copy_from_slice(&i.to_be_bytes());
                        key
                    })
                    .collect(),
                ..stage29.clone()
            },
        ] {
            assert!(canonical(&broken).is_err());
        }
    }
}
