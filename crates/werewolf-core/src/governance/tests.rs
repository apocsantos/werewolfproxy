use super::*;

/// Fixed seeds: governance key `i` is `[i + 1; 32]`, members are
/// `[0xA0 + i; 32]`. Ed25519 signing is deterministic, so every byte below
/// is reproducible.
fn gov(i: u8) -> SigningKey {
    SigningKey::from_bytes(&[i + 1; 32])
}
fn gov_keys() -> Vec<Key> {
    (0..10).map(|i| gov(i).verifying_key().to_bytes()).collect()
}
fn member(i: u8) -> Key {
    SigningKey::from_bytes(&[0xA0 + i; 32])
        .verifying_key()
        .to_bytes()
}
const NETWORK: [u8; 32] = [0x77; 32];

fn signed(core: Core, signers: &[u8]) -> Statement {
    let signatures = signers.iter().map(|&i| (i, sign(&core, &gov(i)))).collect();
    Statement { core, signatures }
}

fn genesis_core() -> Core {
    Core {
        network_id: NETWORK,
        sequence: 0,
        previous: [0; 32],
        body: Body::Genesis {
            threshold: 7,
            keys: gov_keys(),
        },
    }
}

fn genesis() -> Statement {
    signed(genesis_core(), &[0, 1, 2, 3, 4, 5, 6])
}

fn head() -> Head {
    let g = genesis();
    match apply(None, &g, Some(&g.core.hash())).unwrap() {
        Outcome::Applied(head) => head,
        other => panic!("{other:?}"),
    }
}

fn next(head: &Head, body: Body) -> Core {
    Core {
        network_id: head.network_id,
        sequence: head.sequence + 1,
        previous: head.hash,
        body,
    }
}

fn applied(outcome: Result<Outcome, GovError>) -> Head {
    match outcome {
        Ok(Outcome::Applied(head)) => head,
        other => panic!("{other:?}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn revocation(head: &Head) -> Statement {
    let mut keys = vec![member(1), member(0)];
    keys.sort();
    signed(
        next(head, Body::Revocation(keys)),
        &[1, 2, 3, 4, 5, 6, 7, 8],
    )
}

/// Re-signs `core` after a body change, as a real ceremony would.
fn resign(statement: &Statement, core: Core) -> Statement {
    let signers: Vec<u8> = statement.signatures.iter().map(|(i, _)| *i).collect();
    signed(core, &signers)
}

#[test]
fn vectors_are_stable() {
    let g = genesis();
    let core = g.core.encode();
    // version, network ID, sequence 0, zero previous, kind 0, body length 322
    assert_eq!(core[0], 1);
    assert_eq!(&core[1..33], &NETWORK);
    assert_eq!(&core[33..73], &[0; 40]);
    assert_eq!(core[73], 0);
    assert_eq!(u16::from_be_bytes([core[74], core[75]]), 2 + 10 * 32);
    assert_eq!(&core[76..78], &[7, 10]);
    assert_eq!(
        hex(&g.core.hash()),
        "2fb0876a1ef95d60c2fc8367e05831d513ddf3c12a004403dee5c084870351cc"
    );
    assert_eq!(
        hex(blake3::hash(&g.encode()).as_bytes()),
        "fed8e77f22882ba7fde801e429f9a4c919118e6e40bc3ce82a26e2561bcd4cf6"
    );
    let h = head();
    assert_eq!(
        hex(&revocation(&h).core.hash()),
        "94f60e8fc612cab604ae9138c4ead0881798256cc2a5bdefd8ffe6480eefc8dd"
    );
    let rotation = next(&h, Body::Rotation(vec![(9, member(9))]));
    assert_eq!(
        hex(&rotation.hash()),
        "2821d1f7a460679794059d76cf37f86dd269b50d2929421d4da0977e48e96967"
    );
    // Signing input and hash use different domains over the same core.
    assert!(g
        .core
        .signing_input()
        .starts_with(b"werewolfproxy/governance/v1/sign\0"));
    assert!(g.core.signing_input().ends_with(&core));
}

#[test]
fn every_kind_round_trips() {
    let h = head();
    for statement in [
        genesis(),
        revocation(&h),
        signed(
            next(&h, Body::Rotation(vec![(2, member(2)), (9, member(9))])),
            &[0, 1, 2, 3, 4, 5, 6, 9],
        ),
    ] {
        assert_eq!(Statement::decode(&statement.encode()), Ok(statement));
    }
}

#[test]
fn encoding_is_strict() {
    let bytes = genesis().encode();
    let mut bad = bytes.clone();
    bad[0] = 2;
    assert_eq!(Statement::decode(&bad), Err(GovError::UnknownVersion));
    let mut bad = bytes.clone();
    bad[73] = 5; // kinds 0-4 are defined (Stage28 and Stage29)
    assert_eq!(Statement::decode(&bad), Err(GovError::UnknownKind));
    let mut bad = bytes.clone();
    bad.push(0);
    assert_eq!(Statement::decode(&bad), Err(GovError::Malformed));
    for len in 0..bytes.len() {
        assert!(Statement::decode(&bytes[..len]).is_err(), "{len}");
    }
    // A body length that does not match the body's own structure.
    for delta in [-1i32, 1] {
        let mut bad = bytes.clone();
        let len = u16::from_be_bytes([bad[74], bad[75]]) as i32 + delta;
        bad[74..76].copy_from_slice(&(len as u16).to_be_bytes());
        assert!(Statement::decode(&bad).is_err(), "{delta}");
    }
    assert_eq!(
        Statement::decode(&vec![1; MAX_STATEMENT_LEN + 1]),
        Err(GovError::TooLarge)
    );
}

#[test]
fn genesis_is_pinned_and_strictly_signed() {
    let g = genesis();
    let hash = g.core.hash();
    assert_eq!(apply(None, &g, None), Err(GovError::GenesisMismatch));
    assert_eq!(
        apply(None, &g, Some(&[0; 32])),
        Err(GovError::GenesisMismatch)
    );
    assert!(matches!(
        apply(None, &g, Some(&hash)),
        Ok(Outcome::Applied(_))
    ));

    let core = genesis_core();
    let pinned = |s: &Statement| apply(None, s, Some(&s.core.hash()));
    // Six signatures, duplicate, unordered or out-of-range indices.
    for signers in [
        &[0, 1, 2, 3, 4, 5][..],
        &[0, 1, 2, 3, 4, 5, 5],
        &[1, 0, 2, 3, 4, 5, 6],
    ] {
        let mut s = signed(core.clone(), &[0, 1, 2, 3, 4, 5, 6]);
        s.signatures = signers.iter().map(|&i| (i, sign(&core, &gov(i)))).collect();
        assert_eq!(pinned(&s), Err(GovError::Signatures), "{signers:?}");
    }
    let mut s = signed(core.clone(), &[0, 1, 2, 3, 4, 5, 6]);
    s.signatures.push((10, [0; 64]));
    assert_eq!(pinned(&s), Err(GovError::Signatures));
    // One bad signature among eight good ones rejects the statement.
    let mut s = signed(core.clone(), &[0, 1, 2, 3, 4, 5, 6, 7]);
    s.signatures[3].1[0] ^= 1;
    assert_eq!(pinned(&s), Err(GovError::Signatures));
    // A signature by the right index but the wrong key.
    let mut s = signed(core.clone(), &[0, 1, 2, 3, 4, 5, 6]);
    s.signatures[6].1 = sign(&core, &gov(9));
    assert_eq!(pinned(&s), Err(GovError::Signatures));
    // A malleated (non-canonical S) signature never verifies.
    let mut s = signed(core.clone(), &[0, 1, 2, 3, 4, 5, 6]);
    let l: [u8; 32] = [
        0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde,
        0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
    ];
    let mut carry = 0u16;
    for (byte, add) in s.signatures[0].1[32..].iter_mut().zip(l) {
        let sum = *byte as u16 + add as u16 + carry;
        *byte = sum as u8;
        carry = sum >> 8;
    }
    assert_eq!(pinned(&s), Err(GovError::Signatures));

    // Body rules: exactly 7-of-10, distinct strong keys.
    let with_body = |body| Core {
        body,
        ..genesis_core()
    };
    let mut keys = gov_keys();
    for body in [
        Body::Genesis {
            threshold: 6,
            keys: gov_keys(),
        },
        Body::Genesis {
            threshold: 7,
            keys: gov_keys()[..9].to_vec(),
        },
        {
            keys[9] = keys[0];
            Body::Genesis {
                threshold: 7,
                keys: keys.clone(),
            }
        },
        {
            let mut keys = gov_keys();
            keys[9] = small_order_point();
            Body::Genesis { threshold: 7, keys }
        },
    ] {
        let s = signed(with_body(body), &[0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(pinned(&s), Err(GovError::BodyRule));
    }
    // Only a genesis can start a chain, and only at sequence 0.
    assert_eq!(
        apply(None, &revocation(&head()), None),
        Err(GovError::NoGenesis)
    );
    let late = signed(
        Core {
            sequence: 1,
            ..genesis_core()
        },
        &[0, 1, 2, 3, 4, 5, 6],
    );
    assert_eq!(
        apply(None, &late, Some(&late.core.hash())),
        Err(GovError::OutOfOrder)
    );
}

fn small_order_point() -> Key {
    // The identity point: valid encoding, order 1.
    let mut key = [0; 32];
    key[0] = 1;
    assert!(VerifyingKey::from_bytes(&key).unwrap().is_weak());
    key
}

#[test]
fn revocation_extends_the_chain_once() {
    let h = head();
    let r = revocation(&h);
    let after = applied(apply(Some(&h), &r, None));
    let mut expected = vec![member(0), member(1)];
    expected.sort();
    assert_eq!(after.revoked, expected);
    assert_eq!((after.sequence, after.hash), (1, r.core.hash()));
    assert_eq!(after.applied, vec![h.hash, r.core.hash()]);
    // Replays are recognised and change nothing, with any signature set.
    assert_eq!(apply(Some(&after), &r, None), Ok(Outcome::AlreadyApplied));
    let other_signers = resign(&r, r.core.clone());
    let other_signers = signed(other_signers.core, &[3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(
        apply(Some(&after), &other_signers, None),
        Ok(Outcome::AlreadyApplied)
    );
    assert_eq!(
        apply(Some(&after), &genesis(), None),
        Ok(Outcome::AlreadyApplied)
    );
    // Chain rules.
    let gap = signed(
        Core {
            sequence: 3,
            ..next(&after, Body::Revocation(vec![member(2)]))
        },
        &[0, 1, 2, 3, 4, 5, 6],
    );
    assert_eq!(apply(Some(&after), &gap, None), Err(GovError::OutOfOrder));
    let stale = signed(
        Core {
            previous: h.hash,
            ..next(&after, Body::Revocation(vec![member(2)]))
        },
        &[0, 1, 2, 3, 4, 5, 6],
    );
    assert_eq!(
        apply(Some(&after), &stale, None),
        Err(GovError::PreviousMismatch)
    );
    let foreign = signed(
        Core {
            network_id: [1; 32],
            ..next(&after, Body::Revocation(vec![member(2)]))
        },
        &[0, 1, 2, 3, 4, 5, 6],
    );
    assert_eq!(
        apply(Some(&after), &foreign, None),
        Err(GovError::WrongNetwork)
    );
    let second_genesis = signed(
        Core {
            sequence: 2,
            previous: after.hash,
            ..genesis_core()
        },
        &[0, 1, 2, 3, 4, 5, 6],
    );
    assert_eq!(
        apply(Some(&after), &second_genesis, None),
        Err(GovError::OutOfOrder)
    );
}

#[test]
fn revocation_lists_are_canonical() {
    let h = head();
    let invalid_point = (0u8..=255)
        .map(|y| {
            let mut key = [0u8; 32];
            key[0] = y;
            key[31] = 0x7f;
            key
        })
        .find(|key| VerifyingKey::from_bytes(key).is_err())
        .expect("some y is not on the curve");
    let mut sorted = [member(0), member(1)];
    sorted.sort();
    let unsorted = vec![sorted[1], sorted[0]];
    let many: Vec<Key> = {
        let mut keys: Vec<Key> = (0..65u8)
            .map(|i| SigningKey::from_bytes(&[i; 32]).verifying_key().to_bytes())
            .collect();
        keys.sort();
        keys
    };
    for keys in [
        vec![],
        unsorted,
        vec![sorted[0], sorted[0]],
        many,
        vec![invalid_point],
    ] {
        let s = signed(
            next(&h, Body::Revocation(keys.clone())),
            &[0, 1, 2, 3, 4, 5, 6],
        );
        assert_eq!(
            apply(Some(&h), &s, None),
            Err(GovError::BodyRule),
            "{}",
            keys.len()
        );
    }
}

#[test]
fn rotation_changes_who_signs_next() {
    let h = head();
    let new_key = SigningKey::from_bytes(&[0x55; 32]);
    let rotation = signed(
        next(
            &h,
            Body::Rotation(vec![(9, new_key.verifying_key().to_bytes())]),
        ),
        &[0, 1, 2, 3, 4, 5, 9],
    );
    let after = applied(apply(Some(&h), &rotation, None));
    assert_eq!(after.keys()[9], new_key.verifying_key().to_bytes());
    // The old key 9 no longer counts; the new one does.
    let core = next(&after, Body::Revocation(vec![member(3)]));
    let old = signed(core.clone(), &[0, 1, 2, 3, 4, 5, 9]);
    assert_eq!(apply(Some(&after), &old, None), Err(GovError::Signatures));
    let mut new = signed(core.clone(), &[0, 1, 2, 3, 4, 5]);
    new.signatures.push((9, sign(&core, &new_key)));
    assert!(matches!(
        apply(Some(&after), &new, None),
        Ok(Outcome::Applied(_))
    ));

    for entries in [
        vec![],
        vec![(9, gov_keys()[0])],             // duplicates a kept key
        vec![(9, gov_keys()[9])],             // no change
        vec![(5, member(5)), (2, member(2))], // unordered
        vec![(2, member(2)), (2, member(3))], // repeated index
        vec![(10, member(5))],                // out of range
        vec![(1, member(1)), (2, member(1))], // duplicates within
        vec![(9, small_order_point())],
    ] {
        let s = signed(
            next(&h, Body::Rotation(entries.clone())),
            &[0, 1, 2, 3, 4, 5, 6],
        );
        assert_eq!(
            apply(Some(&h), &s, None),
            Err(GovError::BodyRule),
            "{entries:?}"
        );
    }
}

#[test]
fn a_validly_signed_conflict_fences_governance() {
    let h = head();
    let first = revocation(&h);
    let after = applied(apply(Some(&h), &first, None));
    // A different statement for the same sequence, signed by the keys in
    // force then, is a fork.
    let conflict = signed(
        next(&h, Body::Revocation(vec![member(5)])),
        &[2, 3, 4, 5, 6, 7, 8],
    );
    let fenced = match apply(Some(&after), &conflict, None) {
        Ok(Outcome::Fork(head)) => head,
        other => panic!("{other:?}"),
    };
    assert!(fenced.forked);
    assert_eq!((fenced.sequence, fenced.hash), (after.sequence, after.hash));
    assert_eq!(
        apply(
            Some(&fenced),
            &signed(
                next(&fenced, Body::Revocation(vec![member(6)])),
                &[0, 1, 2, 3, 4, 5, 6]
            ),
            None
        ),
        Err(GovError::Forked)
    );
    // Without valid signatures a conflict is just rejected.
    let mut forged = conflict.clone();
    forged.signatures[0].1[0] ^= 1;
    assert_eq!(
        apply(Some(&after), &forged, None),
        Err(GovError::Signatures)
    );
    // A different genesis is refused, never a fork: it says nothing about
    // this network's keys.
    let other = signed(
        Core {
            body: Body::Genesis {
                threshold: 7,
                keys: (0..10).map(member).collect(),
            },
            ..genesis_core()
        },
        &[],
    );
    assert_eq!(
        apply(Some(&after), &other, None),
        Err(GovError::GenesisMismatch)
    );
}

#[test]
fn forks_are_judged_by_the_keys_in_force_then() {
    let h = head();
    let new_key = SigningKey::from_bytes(&[0x55; 32]);
    let rotation = signed(
        next(
            &h,
            Body::Rotation(vec![(0, new_key.verifying_key().to_bytes())]),
        ),
        &[0, 1, 2, 3, 4, 5, 6],
    );
    let rotated = applied(apply(Some(&h), &rotation, None));
    let later = applied(apply(Some(&rotated), &revocation(&rotated), None));
    // A conflicting sequence 1 signed by the original keys, including the
    // old key 0, is still evidence of a fork.
    let conflict = signed(
        next(&h, Body::Revocation(vec![member(7)])),
        &[0, 1, 2, 3, 4, 5, 6],
    );
    assert!(matches!(
        apply(Some(&later), &conflict, None),
        Ok(Outcome::Fork(_))
    ));
}

// Stage29: admission, activation, roster and local admission.

const SIGNERS: [u8; 7] = [0, 1, 2, 3, 4, 5, 6];

fn sorted(mut keys: Vec<Key>) -> Vec<Key> {
    keys.sort();
    keys
}

fn step(head: &Head, body: Body) -> Head {
    applied(apply(Some(head), &signed(next(head, body), &SIGNERS), None))
}

fn refused(head: &Head, body: Body) -> GovError {
    apply(Some(head), &signed(next(head, body), &SIGNERS), None).unwrap_err()
}

#[test]
fn stage29_vectors_are_stable() {
    let h = head();
    let admission = next(&h, Body::Admission(sorted(vec![member(0), member(1)])));
    assert_eq!(
        hex(&admission.hash()),
        "6604b37ee667c38833c0167ce1b6843ffbbd3c09cca5d595f1f019ffbe0ecdf6"
    );
    let admitted = step(&h, Body::Admission(sorted(vec![member(0), member(1)])));
    let activation = next(&admitted, Body::Activation);
    // kind 4, body length 0, nothing else.
    assert!(hex(&activation.encode()).ends_with("040000"));
    assert_eq!(
        hex(&activation.hash()),
        "a4cef971dd208735b35031682d5c3cb1427ca9dd00826ea416ff310c32610d2f"
    );
    for body in [Body::Admission(sorted(vec![member(2)])), Body::Activation] {
        let statement = signed(next(&admitted, body), &SIGNERS);
        assert_eq!(Statement::decode(&statement.encode()), Ok(statement));
    }
}

#[test]
fn admission_rules() {
    let h = head();
    let many: Vec<Key> = sorted(
        (0..65u8)
            .map(|i| SigningKey::from_bytes(&[i; 32]).verifying_key().to_bytes())
            .collect(),
    );
    for keys in [
        vec![],
        many.clone(),
        {
            let pair = sorted(vec![member(0), member(1)]);
            vec![pair[1], pair[0]]
        },
        vec![member(0), member(0)],
        vec![small_order_point()],
    ] {
        assert_eq!(
            refused(&h, Body::Admission(keys.clone())),
            GovError::BodyRule,
            "{}",
            keys.len()
        );
    }
    assert_eq!(
        step(&h, Body::Admission(many[..64].to_vec())).roster.len(),
        64
    );
    // Already on the roster.
    let admitted = step(&h, Body::Admission(vec![member(0)]));
    assert_eq!(admitted.roster, vec![member(0)]);
    assert_eq!(
        refused(&admitted, Body::Admission(vec![member(0)])),
        GovError::BodyRule
    );
    // An activation with a body is not an activation.
    let mut with_body = signed(next(&admitted, Body::Activation), &SIGNERS).encode();
    let body_len = 74;
    with_body[body_len..body_len + 2].copy_from_slice(&1u16.to_be_bytes());
    with_body.insert(76, 0);
    assert!(Statement::decode(&with_body).is_err());
    // Only once.
    let active = step(&admitted, Body::Activation);
    assert_eq!(active.activated_at, Some(active.sequence));
    assert_eq!(refused(&active, Body::Activation), GovError::BodyRule);
}

#[test]
fn the_roster_is_bounded_at_1024() {
    let mut h = head();
    let key = |i: u32| {
        let mut seed = [0x33u8; 32];
        seed[..4].copy_from_slice(&i.to_be_bytes());
        SigningKey::from_bytes(&seed).verifying_key().to_bytes()
    };
    for batch in 0..16u32 {
        h = step(
            &h,
            Body::Admission(sorted((0..64).map(|i| key(batch * 64 + i)).collect())),
        );
    }
    assert_eq!(h.roster.len(), 1024);
    assert_eq!(
        refused(&h, Body::Admission(vec![key(5000)])),
        GovError::BodyRule
    );
    // Revoking one frees a place, but the revoked key can never return.
    let gone = h.roster[0];
    h = step(&h, Body::Revocation(vec![gone]));
    assert_eq!(h.roster.len(), 1023);
    assert_eq!(refused(&h, Body::Admission(vec![gone])), GovError::BodyRule);
    assert_eq!(
        step(&h, Body::Admission(vec![key(5000)])).roster.len(),
        1024
    );
}

#[test]
fn revocation_removes_from_the_roster_and_admission_works_after_activation() {
    let h = step(&head(), Body::Admission(sorted(vec![member(0), member(1)])));
    let h = step(&h, Body::Activation);
    let h = step(&h, Body::Revocation(vec![member(0)]));
    assert_eq!(h.roster, vec![member(1)]);
    assert_eq!(h.revoked, vec![member(0)]);
    let h = step(&h, Body::Admission(vec![member(2)]));
    assert_eq!(h.roster, sorted(vec![member(1), member(2)]));
}

/// Owner scenario 1: a node admitted after activation.
#[test]
fn scenario_a_node_admitted_after_activation() {
    let x = member(9);
    let other = member(1);
    // Fresh node X applies the same chain as everyone, in order.
    let genesis_head = head();
    assert!(
        genesis_head.admits(&x),
        "before activation every node is eligible"
    );
    let with_other = step(&genesis_head, Body::Admission(vec![other]));
    // Activation is a network-chain fact: X applies it and advances.
    let activated = step(&with_other, Body::Activation);
    assert_eq!(activated.sequence, 2);
    assert_eq!(activated.activated_at, Some(2));
    assert!(!activated.admits(&x), "X is fenced: not on the roster");
    assert!(activated.admits(&other));
    // A later admission of X makes it a member, nothing skipped or
    // reordered: X's head equals the one every node computes.
    let admitted = step(&activated, Body::Admission(vec![x]));
    assert_eq!(admitted.sequence, 3);
    assert_eq!(admitted.applied.len(), 4);
    assert!(admitted.admits(&x));
    // Skipping straight to the admission is refused.
    let skipped = signed(next(&activated, Body::Admission(vec![x])), &SIGNERS);
    assert_eq!(
        apply(Some(&with_other), &skipped, None),
        Err(GovError::OutOfOrder)
    );
    // A revocation of X, or a new Pelt not on the roster, fences X again.
    let revoked = step(&admitted, Body::Revocation(vec![x]));
    assert!(!revoked.admits(&x));
    assert!(!admitted.admits(&member(8)));
}

/// Owner scenario 2: revocation is permanent.
#[test]
fn scenario_revocation_is_permanent() {
    let x = member(4);
    let h = step(&head(), Body::Admission(vec![x]));
    let h = step(&h, Body::Revocation(vec![x]));
    assert_eq!(refused(&h, Body::Admission(vec![x])), GovError::BodyRule);
    // Still refused after any number of later statements, before and after
    // activation, alone or alongside other keys.
    let h = step(&h, Body::Admission(vec![member(5)]));
    let h = step(&h, Body::Activation);
    assert_eq!(refused(&h, Body::Admission(vec![x])), GovError::BodyRule);
    assert_eq!(
        refused(&h, Body::Admission(sorted(vec![x, member(6)]))),
        GovError::BodyRule
    );
    assert!(!h.admits(&x));
}
