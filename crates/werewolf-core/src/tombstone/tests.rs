use super::*;
use crate::governance::{sign, Core, Key};
use ed25519_dalek::SigningKey;

const SOFTWARE: &str = "werewolfd 0.1.0-rc1";

fn gov(i: u8) -> SigningKey {
    SigningKey::from_bytes(&[i + 1; 32])
}

fn statement(body: Body) -> Statement {
    let core = Core {
        network_id: [0x77; 32],
        sequence: 4,
        previous: [0x22; 32],
        body,
    };
    let signatures = (0..7u8).map(|i| (i, sign(&core, &gov(i)))).collect();
    Statement { core, signatures }
}

fn death() -> Tombstone {
    Tombstone::death(statement(Body::Death), Destruction::Incomplete, SOFTWARE).unwrap()
}

fn document(tombstone: &Tombstone) -> serde_json::Value {
    serde_json::from_slice(&tombstone.encode().unwrap()).unwrap()
}

fn parse(value: &serde_json::Value) -> io::Result<Tombstone> {
    Tombstone::parse(&serde_json::to_vec(value).unwrap())
}

#[test]
fn both_kinds_round_trip_with_one_encoding() {
    for tombstone in [
        death(),
        Tombstone::death(statement(Body::Death), Destruction::Complete, SOFTWARE).unwrap(),
        Tombstone::retirement(Destruction::Incomplete, SOFTWARE).unwrap(),
        Tombstone::retirement(Destruction::Complete, SOFTWARE).unwrap(),
    ] {
        let bytes = tombstone.encode().unwrap();
        assert!(bytes.len() <= MAX_LEN);
        assert_eq!(Tombstone::parse(&bytes).unwrap(), tombstone);
    }
    let bytes = String::from_utf8(death().encode().unwrap()).unwrap();
    assert!(bytes.starts_with(
        "{\"format\":\"werewolf-tombstone\",\"version\":1,\"kind\":\"death\",\"network_id\":"
    ));
    assert!(
        bytes.ends_with(",\"destruction\":\"incomplete\",\"software\":\"werewolfd 0.1.0-rc1\"}")
    );
    let retirement = Tombstone::retirement(Destruction::Complete, SOFTWARE).unwrap();
    assert_eq!(
        String::from_utf8(retirement.encode().unwrap()).unwrap(),
        "{\"format\":\"werewolf-tombstone\",\"version\":1,\"kind\":\"retirement\",\
         \"destruction\":\"complete\",\"software\":\"werewolfd 0.1.0-rc1\"}"
    );
}

#[test]
fn the_death_fields_agree_with_the_statement() {
    let tombstone = death();
    let doc = document(&tombstone);
    let core = &tombstone.death_statement().unwrap().core;
    assert_eq!(doc["network_id"], hex(&core.network_id));
    assert_eq!(doc["terminal_sequence"], core.sequence);
    assert_eq!(doc["terminal_hash"], hex(&core.hash()));
    for (field, value) in [
        ("network_id", serde_json::json!(hex(&[0x78; 32]))),
        ("terminal_sequence", serde_json::json!(5)),
        ("terminal_hash", serde_json::json!(hex(&[0; 32]))),
    ] {
        let mut bad = doc.clone();
        bad[field] = value;
        assert!(parse(&bad).is_err(), "{field}");
    }
}

#[test]
fn the_schema_is_strict() {
    let doc = document(&death());
    // An unknown field, a missing field, an unknown kind or destruction.
    let mut extra = doc.clone();
    extra["timestamp"] = 0.into();
    assert!(parse(&extra).is_err());
    let mut missing = doc.clone();
    missing.as_object_mut().unwrap().remove("death_statement");
    assert!(parse(&missing).is_err());
    for (field, value) in [
        ("kind", "funeral"),
        ("destruction", "partial"),
        ("format", "tombstone"),
    ] {
        let mut bad = doc.clone();
        bad[field] = value.into();
        assert!(parse(&bad).is_err(), "{field}");
    }
    let mut version = doc.clone();
    version["version"] = 2.into();
    assert!(parse(&version).is_err());
    // A retirement carries no chain data.
    let mut retirement = document(&Tombstone::retirement(Destruction::Complete, SOFTWARE).unwrap());
    retirement["network_id"] = hex(&[0x77; 32]).into();
    assert!(parse(&retirement).is_err());
    // Only the canonical bytes parse: no whitespace, no reordering.
    let bytes = death().encode().unwrap();
    let mut spaced = bytes.clone();
    spaced.insert(1, b' ');
    assert!(Tombstone::parse(&spaced).is_err());
    let reordered = serde_json::to_vec(&doc).unwrap(); // serde_json sorts keys
    assert_ne!(reordered, bytes);
    assert!(Tombstone::parse(&reordered).is_err());
    // A duplicated field.
    let text = String::from_utf8(bytes).unwrap();
    let duplicated = text.replacen("\"version\":1,", "\"version\":1,\"version\":1,", 1);
    assert!(Tombstone::parse(duplicated.as_bytes()).is_err());
    // More than 8 KiB.
    assert!(Tombstone::parse(&vec![b' '; MAX_LEN + 1]).is_err());
}

#[test]
fn only_a_death_statement_makes_a_death_tombstone() {
    for body in [Body::Activation, Body::Revocation(vec![[0x33; 32]])] {
        assert!(Tombstone::death(statement(body), Destruction::Incomplete, SOFTWARE).is_err());
    }
    let mut doc = document(&death());
    doc["death_statement"] = STANDARD.encode(statement(Body::Activation).encode()).into();
    assert!(parse(&doc).is_err());
    let mut garbage = document(&death());
    garbage["death_statement"] = "AAAA".into();
    assert!(parse(&garbage).is_err());
}

#[test]
fn software_is_short_printable_ascii() {
    for software in ["", "werewolfd ✓", "werewolfd\n0.1", &"x".repeat(65)] {
        assert!(
            Tombstone::retirement(Destruction::Complete, software).is_err(),
            "{software:?}"
        );
    }
    assert!(Tombstone::retirement(Destruction::Complete, &"x".repeat(64)).is_ok());
}

/// Section 5 "proof", the mechanical part: no governance public key, in
/// raw, hex or base64 form, appears in a death Tombstone, and its only
/// statement-derived content is the death statement itself.
#[test]
fn a_death_tombstone_carries_no_governance_key() {
    let bytes = death().encode().unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    let keys: Vec<Key> = (0..10).map(|i| gov(i).verifying_key().to_bytes()).collect();
    let decoded = STANDARD
        .decode(document(&death())["death_statement"].as_str().unwrap())
        .unwrap();
    for key in &keys {
        assert!(!text.contains(&hex(key)));
        assert!(!text.contains(&STANDARD.encode(key)));
        assert!(!bytes.windows(32).any(|w| w == key));
        assert!(!decoded.windows(32).any(|w| w == key));
    }
}
