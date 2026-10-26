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
    bad[73] = 6; // kinds 0-5 are defined (Stage28, Stage29 and Stage30)
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

// Stage30: death, the terminal rule and death on a forked node.

#[test]
fn stage30_vectors_are_stable() {
    let h = head();
    let admitted = step(&h, Body::Admission(sorted(vec![member(0), member(1)])));
    let active = step(&admitted, Body::Activation);
    let death = next(&active, Body::Death);
    // kind 5, body length 0, nothing else.
    assert!(hex(&death.encode()).ends_with("050000"));
    assert_eq!(
        hex(&death.hash()),
        "15a604c4347110f1caeea4576143bf24ee70ad6d7569ce084170d95a5194cbb2"
    );
    let statement = signed(death, &SIGNERS);
    assert_eq!(Statement::decode(&statement.encode()), Ok(statement));
}

#[test]
fn a_death_body_must_be_empty() {
    let h = head();
    let mut bytes = signed(next(&h, Body::Death), &SIGNERS).encode();
    // body_len sits after version, network ID, sequence, previous and kind.
    let at = 1 + 32 + 8 + 32 + 1;
    assert_eq!(&bytes[at..at + 2], &[0, 0]);
    bytes[at + 1] = 1;
    bytes.insert(at + 2, 0x42);
    assert_eq!(Statement::decode(&bytes), Err(GovError::Malformed));
}

#[test]
fn death_follows_ceremony_or_activation_and_nothing_follows_death() {
    let h = head();
    // From CEREMONY (genesis only) and from ACTIVE.
    let dead = step(&h, Body::Death);
    assert_eq!(dead.dead_at, Some(1));
    let active = step(
        &step(&h, Body::Admission(vec![member(0)])),
        Body::Activation,
    );
    let dead_active = step(&active, Body::Death);
    assert_eq!(dead_active.dead_at, Some(dead_active.sequence));

    // Every kind after death is Terminal, a second death included.
    for body in [
        Body::Death,
        Body::Activation,
        Body::Admission(vec![member(3)]),
        Body::Revocation(vec![member(3)]),
        Body::Rotation(vec![(0, member(4))]),
    ] {
        assert_eq!(refused(&dead, body), GovError::Terminal);
    }
    // Re-presenting the death, or an earlier statement, is already applied.
    let death = signed(next(&h, Body::Death), &SIGNERS);
    assert_eq!(
        apply(Some(&dead), &death, None),
        Ok(Outcome::AlreadyApplied)
    );
    assert_eq!(
        apply(Some(&dead), &genesis(), None),
        Ok(Outcome::AlreadyApplied)
    );
    // A conflicting statement at the death's sequence is refused, not
    // recorded: the chain has ended.
    let conflict = signed(next(&h, Body::Revocation(vec![member(5)])), &SIGNERS);
    assert_eq!(apply(Some(&dead), &conflict, None), Err(GovError::Terminal));
    // A death signed by six keys is no death.
    let short = signed(next(&h, Body::Death), &[0, 1, 2, 3, 4, 5]);
    assert_eq!(apply(Some(&h), &short, None), Err(GovError::Signatures));
}

#[test]
fn a_node_that_has_not_died_sees_a_conflicting_death_as_a_fork() {
    let h = head();
    let applied_first = step(&h, Body::Revocation(vec![member(5)]));
    let death = signed(next(&h, Body::Death), &SIGNERS);
    assert!(matches!(
        apply(Some(&applied_first), &death, None),
        Ok(Outcome::Fork(_))
    ));
}

#[test]
fn death_on_a_forked_node_never_resolves_the_fork() {
    let h = head();
    let first = step(&h, Body::Revocation(vec![member(5)]));
    let conflict = signed(next(&h, Body::Revocation(vec![member(6)])), &SIGNERS);
    let fenced = match apply(Some(&first), &conflict, None) {
        Ok(Outcome::Fork(head)) => head,
        other => panic!("{other:?}"),
    };
    // L1: only a death that chains to the node's own head is accepted.
    assert_eq!(
        refused(&fenced, Body::Revocation(vec![member(7)])),
        GovError::Forked
    );
    assert_eq!(refused(&fenced, Body::Activation), GovError::Forked);
    let off_head = signed(
        Core {
            previous: [0x11; 32],
            ..next(&fenced, Body::Death)
        },
        &SIGNERS,
    );
    assert_eq!(apply(Some(&fenced), &off_head, None), Err(GovError::Forked));
    let gap = signed(
        Core {
            sequence: fenced.sequence + 2,
            ..next(&fenced, Body::Death)
        },
        &SIGNERS,
    );
    assert_eq!(apply(Some(&fenced), &gap, None), Err(GovError::Forked));
    let short = signed(next(&fenced, Body::Death), &[0, 1, 2, 3, 4, 5]);
    assert_eq!(
        apply(Some(&fenced), &short, None),
        Err(GovError::Signatures)
    );

    let dead = step(&fenced, Body::Death);
    // I1: the fork stays recorded, no branch is chosen, and the history
    // before the death is exactly the forked node's own.
    assert!(dead.forked);
    assert_eq!(dead.dead_at, Some(dead.sequence));
    assert_eq!(
        &dead.applied[..fenced.applied.len()],
        fenced.applied.as_slice()
    );
    assert_eq!(
        (
            dead.revoked.clone(),
            dead.roster.clone(),
            dead.key_epochs.clone()
        ),
        (
            fenced.revoked.clone(),
            fenced.roster.clone(),
            fenced.key_epochs.clone()
        )
    );
    // And nothing follows it, on a forked node as anywhere.
    assert_eq!(
        refused(&dead, Body::Revocation(vec![member(7)])),
        GovError::Terminal
    );
}

// Stage30 key check (spec section 2).

fn challenge_for(head: &Head, bytes: [u8; 32]) -> check::Challenge {
    check::Challenge {
        format: "werewolf-key-check".into(),
        version: 1,
        network_id: hex(&head.network_id),
        challenge: hex(&bytes),
    }
}

fn answers(challenge: &check::Challenge, indices: &[u8]) -> Vec<check::Answer> {
    indices
        .iter()
        .map(|&i| challenge.answer(&gov(i), i).unwrap())
        .collect()
}

#[test]
fn key_check_vectors_are_stable() {
    let h = head();
    let challenge = challenge_for(&h, [0x33; 32]);
    let input = check::input_for_tests(&NETWORK, &[0x33; 32]);
    assert!(input.starts_with(b"werewolfproxy/governance/v1/key-check\0"));
    assert_eq!(input.len(), 38 + 32 + 32);
    let answer = challenge.answer(&gov(0), 0).unwrap();
    assert_eq!(
        answer.signature,
        "0f375609ab7e10bca4cd9f612f170ff0ad6506b8a443a9fa0561876feebe7789\
         ed6bb329b095bc71812dbad65377ed8e31e6246684163d7cd5c8e5406f723706"
    );
    let json = serde_json::to_vec(&answer).unwrap();
    assert_eq!(check::Answer::parse(&json).unwrap(), answer);
}

#[test]
fn key_check_signatures_never_cross_with_statement_signatures() {
    let h = head();
    let challenge = challenge_for(&h, [0x44; 32]);
    // A statement signature presented as a check answer fails...
    let statement = signed(next(&h, Body::Death), &[0]);
    let mut answer = challenge.answer(&gov(0), 0).unwrap();
    answer.signature = hex(&statement.signatures[0].1);
    assert!(challenge.verify(&h, &[answer]).is_err());
    // ...and a check signature presented as a statement signature fails.
    let answers = answers(&challenge, &SIGNERS);
    let mut death = signed(next(&h, Body::Death), &SIGNERS);
    let forged: [u8; 64] = (0..64)
        .map(|i| u8::from_str_radix(&answers[0].signature[2 * i..2 * i + 2], 16).unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    death.signatures[0].1 = forged;
    assert_eq!(apply(Some(&h), &death, None), Err(GovError::Signatures));
}

#[test]
fn key_check_gives_a_lower_bound_only() {
    let h = head();
    let challenge = challenge_for(&h, [0x55; 32]);
    let (indices, verdict) = challenge
        .verify(&h, &answers(&challenge, &[9, 0, 4, 1, 2, 3]))
        .unwrap();
    assert_eq!(indices, vec![0, 1, 2, 3, 4, 9]);
    assert_eq!(verdict, check::Verdict::Unverified);
    let (indices, verdict) = challenge
        .verify(&h, &answers(&challenge, &SIGNERS))
        .unwrap();
    assert_eq!(indices.len(), 7);
    assert_eq!(verdict, check::Verdict::AtLeast7Usable);
    assert_eq!(verdict.label(), "at_least_7_usable");
    assert_eq!(check::Verdict::Unverified.label(), "unverified");
    // No answers is unverified, never "unrecoverable".
    assert_eq!(
        challenge.verify(&h, &[]).unwrap().1,
        check::Verdict::Unverified
    );
}

#[test]
fn key_check_answers_are_bound_and_distinct() {
    let h = head();
    let challenge = challenge_for(&h, [0x66; 32]);
    let other = challenge_for(&h, [0x67; 32]);
    // An answer to another challenge.
    let stray = other.answer(&gov(1), 1).unwrap();
    let mut mixed = answers(&challenge, &[0]);
    mixed.push(stray);
    assert!(challenge.verify(&h, &mixed).is_err());
    // A duplicate index.
    let twice = answers(&challenge, &[2, 2]);
    assert!(challenge.verify(&h, &twice).is_err());
    // An answer signed by the wrong key for its index.
    let mut wrong = challenge.answer(&gov(3), 3).unwrap();
    wrong.index = 4;
    assert!(challenge.verify(&h, &[wrong]).is_err());
    // Index out of range.
    assert!(challenge.answer(&gov(0), 10).is_err());
    // Another network.
    let mut elsewhere = challenge.clone();
    elsewhere.network_id = hex(&[0x78; 32]);
    assert!(elsewhere.verify(&h, &[]).is_err());
    // After a rotation, the replaced key no longer answers for its index.
    let rotated = step(&h, Body::Rotation(vec![(0, member(9))]));
    assert!(challenge
        .verify(&rotated, &answers(&challenge, &[0]))
        .is_err());
}

#[test]
fn key_check_files_are_strict() {
    let h = head();
    let challenge = challenge_for(&h, [0x77; 32]);
    let json = serde_json::to_value(&challenge).unwrap();
    assert!(check::Challenge::parse(json.to_string().as_bytes()).is_ok());
    let mut extra = json.clone();
    extra["note"] = "x".into();
    assert!(check::Challenge::parse(extra.to_string().as_bytes()).is_err());
    let mut upper = json.clone();
    upper["challenge"] = hex(&[0xAB; 32]).to_uppercase().into();
    assert!(check::Challenge::parse(upper.to_string().as_bytes()).is_err());
    let mut short = json.clone();
    short["network_id"] = "77".into();
    assert!(check::Challenge::parse(short.to_string().as_bytes()).is_err());
    let answer = serde_json::to_value(challenge.answer(&gov(0), 0).unwrap()).unwrap();
    let mut format = answer.clone();
    format["format"] = "werewolf-key-check".into();
    assert!(check::Answer::parse(format.to_string().as_bytes()).is_err());
}
