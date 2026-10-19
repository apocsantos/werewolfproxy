use super::*;
use crate::discovery::CACHE_LIMIT;
use werewolf_core::pelt::generate_identity;

fn record(identity: &PeltIdentity, seq: u64, expires_in: u64) -> ContactRecord {
    let now = unix_now();
    ContactRecord::sign(
        identity,
        vec!["quic://203.0.113.7:9560".into()],
        seq,
        now + expires_in,
        now,
    )
    .unwrap()
}

#[test]
fn advertise_file_is_strict() {
    assert_eq!(
        parse_advertise(br#"{"version":1,"addresses":["tcp://192.0.2.1:9560"]}"#).unwrap(),
        vec!["tcp://192.0.2.1:9560".to_string()]
    );
    assert!(parse_advertise(br#"{"version":1,"addresses":[]}"#)
        .unwrap()
        .is_empty());
    for bad in [
        &br#"{"version":2,"addresses":[]}"#[..],
        br#"{"version":1,"addresses":["https://192.0.2.1:9560"]}"#,
        br#"{"version":1,"addresses":["tcp://wolf.example:9560"]}"#,
        br#"{"version":1,"addresses":[],"extra":true}"#,
        b"{not json",
    ] {
        assert!(parse_advertise(bad).is_err());
    }
}

#[test]
fn cache_file_drops_bad_records_and_tolerates_corruption() {
    let (a, b) = (generate_identity(), generate_identity());
    let good = record(&a, 1, 3600);
    let newer = record(&a, 2, 3600);
    let other = record(&b, 1, 3600);
    let mut forged = other.encode();
    forged[40] ^= 1;
    let file = CacheFile {
        version: 1,
        records: vec![
            STANDARD.encode(good.encode()),
            STANDARD.encode(newer.encode()),
            STANDARD.encode(other.encode()),
            STANDARD.encode(forged),
            "not base64!".into(),
        ],
    };
    let bytes = serde_json::to_vec(&file).unwrap();

    let loaded = parse_cache(&bytes, unix_now());
    assert_eq!(loaded.len(), 2);
    assert!(loaded.contains(&newer) && loaded.contains(&other));
    // Expired by the time the cache is read again.
    assert!(parse_cache(&bytes, unix_now() + 7200).is_empty());
    assert!(parse_cache(b"{not json", unix_now()).is_empty());
    assert!(parse_cache(br#"{"version":9,"records":[]}"#, unix_now()).is_empty());
}

#[test]
fn learn_keeps_newest_skips_own_and_evicts_strangers_first() {
    let own = generate_identity();
    let member = generate_identity();
    let member_key = record(&member, 1, 3600).public_key;
    let now = unix_now();
    let strangers: Vec<_> = (0..CACHE_LIMIT)
        .map(|_| record(&generate_identity(), 1, 7200))
        .collect();
    let contacts = Contacts {
        known: vec![record(&member, 1, 3600)],
        ..Default::default()
    };

    let merged = contacts.learn(
        strangers
            .iter()
            .cloned()
            .chain([record(&own, 1, 3600), record(&member, 5, 60)]),
        &record(&own, 1, 60).public_key,
        &[member_key],
        now,
    );

    assert_eq!(merged.len(), CACHE_LIMIT);
    assert_eq!(merged[0].public_key, member_key);
    assert_eq!(merged[0].seq, 5);
    assert!(merged.iter().all(|r| r.public_key != hs_key(&own)));
}

#[test]
fn own_record_is_resigned_with_increasing_seq() {
    let identity = generate_identity();
    let now = unix_now();
    let mut contacts = Contacts::default();

    contacts.refresh_own(&identity, now);
    assert!(contacts.own.is_none());

    contacts.advertised = vec!["quic://192.0.2.1:9560".into()];
    contacts.refresh_own(&identity, now);
    let first = contacts.own.clone().unwrap();
    assert_eq!(first.seq, now);
    contacts.refresh_own(&identity, now);
    assert_eq!(contacts.own.as_ref(), Some(&first));

    // An address change in the same second still raises seq.
    contacts.advertised = vec!["tcp://192.0.2.1:9560".into()];
    contacts.refresh_own(&identity, now);
    let second = contacts.own.clone().unwrap();
    assert!(second.supersedes(&first));
    assert!(second.verify(now).is_ok());

    // Close to expiry, the same addresses are signed again.
    let later = second.expires_at - 60;
    contacts.refresh_own(&identity, later);
    assert!(contacts.own.as_ref().unwrap().supersedes(&second));

    contacts.advertised.clear();
    contacts.refresh_own(&identity, now);
    assert!(contacts.own.is_none());
}

#[test]
fn target_accepts_node_id_or_public_key_only() {
    let identity = generate_identity();
    let key = hs_key(&identity);
    let id = NodeId::from_public_key(&key);

    assert_eq!(parse_target(&id.to_hex()), Some(id.clone()));
    assert_eq!(parse_target(&identity.public_key_b64), Some(id));
    assert_eq!(parse_target(""), None);
    assert_eq!(parse_target(&"g".repeat(64)), None);
    assert_eq!(parse_target("wolf-b"), None);
}
