use super::*;
use crate::handshake::{parse, verify, QuicOpen, TcpChallenge, TcpOpen};
use werewolf_core::pelt::generate_identity;

const NOW: u64 = 1_800_000_000;
const BINDING: [u8; 32] = [3; 32];

fn record(identity: &PeltIdentity, expires_at: u64) -> ContactRecord {
    ContactRecord::sign(
        identity,
        vec!["quic://203.0.113.7:9560".into()],
        1,
        expires_at,
        NOW,
    )
    .unwrap()
}

fn find(k: u8) -> RpcRequest {
    RpcRequest::FindNode {
        target: NodeId([0xab; 32]),
        k,
    }
}

fn challenge(receiver: &PeltIdentity) -> TcpChallenge {
    TcpChallenge {
        cmd: "fang.tcp.challenge".into(),
        protocol: TCP.into(),
        receiver_fingerprint: receiver.fingerprint.clone(),
        challenge: hex(&[9; 32]),
    }
}

fn line<T: Serialize>(value: &T) -> Vec<u8> {
    let mut data = serde_json::to_vec(value).unwrap();
    data.push(b'\n');
    data
}

#[test]
fn quic_rpc_round_trip_returns_verified_records() {
    let (sender, receiver) = (generate_identity(), generate_identity());
    let rpc = QuicRpc::new(&sender, &receiver.fingerprint, &find(2), &BINDING).unwrap();
    let retained = rpc.transcript(&BINDING).unwrap();
    verify(&sender.public_key_b64, &retained, &rpc.signature).unwrap();
    assert_eq!(
        parse::<QuicRpc>(&line(&rpc)).unwrap().request().unwrap(),
        find(2)
    );

    let records = vec![
        record(&generate_identity(), NOW + 3600),
        record(&generate_identity(), NOW + 3600),
    ];
    let result = QuicRpcResult::new(&rpc, &retained, &receiver, &records).unwrap();
    let parsed: QuicRpcResult = parse(&line(&result)).unwrap();

    assert_eq!(parsed.verify(&rpc, &retained, NOW).unwrap(), records);
}

#[test]
fn quic_rpc_signature_is_bound_to_the_connection() {
    let sender = generate_identity();
    let rpc = QuicRpc::new(
        &sender,
        &generate_identity().fingerprint,
        &RpcRequest::Ping,
        &BINDING,
    )
    .unwrap();
    let other = rpc.transcript(&[4; 32]).unwrap();

    assert!(verify(&sender.public_key_b64, &other, &rpc.signature).is_err());
}

#[test]
fn rpc_signatures_never_verify_as_open() {
    let sender = generate_identity();
    let rpc = QuicRpc::new(
        &sender,
        &generate_identity().fingerprint,
        &find(1),
        &BINDING,
    )
    .unwrap();
    let open = QuicOpen {
        cmd: "fang.quic.open".into(),
        protocol: QUIC.into(),
        sender_fingerprint: rpc.sender_fingerprint.clone(),
        receiver_fingerprint: rpc.receiver_fingerprint.clone(),
        nonce: rpc.nonce.clone(),
        remote: "127.0.0.1:8080".into(),
        signature: rpc.signature.clone(),
    };
    let open_transcript = open.transcript(&BINDING).unwrap();

    assert!(verify(&sender.public_key_b64, &open_transcript, &rpc.signature).is_err());
    let retained = rpc.transcript(&BINDING).unwrap();
    assert!(retained
        .windows(b"fang.quic.rpc/v1".len())
        .any(|w| w == b"fang.quic.rpc/v1"));
}

#[test]
fn pre_stage24_receivers_reject_rpc_like_a_malformed_open() {
    // Older receivers parse the first line strictly as OPEN.
    let (sender, receiver) = (generate_identity(), generate_identity());
    let quic = QuicRpc::new(&sender, &receiver.fingerprint, &find(3), &BINDING).unwrap();
    let tcp = TcpRpc::new(&sender, &challenge(&receiver), &RpcRequest::Ping).unwrap();

    assert!(parse::<QuicOpen>(&line(&quic)).is_err());
    assert!(parse::<TcpOpen>(&line(&tcp)).is_err());
}

#[test]
fn request_fields_are_checked_per_operation() {
    let sender = generate_identity();
    let valid = QuicRpc::new(
        &sender,
        &generate_identity().fingerprint,
        &find(5),
        &BINDING,
    )
    .unwrap();
    let mut ping_with_k = QuicRpc::new(
        &sender,
        &generate_identity().fingerprint,
        &RpcRequest::Ping,
        &BINDING,
    )
    .unwrap();
    ping_with_k.k = 1;
    let mut ping_with_target = ping_with_k.clone();
    ping_with_target.k = 0;
    ping_with_target.target = valid.target.clone();
    let mut k_zero = valid.clone();
    k_zero.k = 0;
    let mut k_large = valid.clone();
    k_large.k = MAX_K + 1;
    let mut unknown_op = valid.clone();
    unknown_op.op = "store".into();
    let mut upper_target = valid.clone();
    upper_target.target = valid.target.to_uppercase();
    let mut wrong_cmd = valid.clone();
    wrong_cmd.cmd = "fang.quic.open".into();

    for bad in [
        ping_with_k,
        ping_with_target,
        k_zero,
        k_large,
        unknown_op,
        upper_target,
        wrong_cmd,
    ] {
        assert!(bad.transcript(&BINDING).is_err(), "{bad:?}");
    }
}

#[test]
fn tampered_results_are_rejected() {
    let (sender, receiver) = (generate_identity(), generate_identity());
    let rpc = QuicRpc::new(&sender, &receiver.fingerprint, &find(2), &BINDING).unwrap();
    let retained = rpc.transcript(&BINDING).unwrap();
    let records = vec![
        record(&generate_identity(), NOW + 3600),
        record(&generate_identity(), NOW + 3600),
    ];
    let result = QuicRpcResult::new(&rpc, &retained, &receiver, &records).unwrap();

    let mut nonce = result.clone();
    nonce.nonce = hex(&[0; 16]);
    let mut op = result.clone();
    op.op = "ping".into();
    let mut reordered = result.clone();
    reordered.records.reverse();
    let mut impostor = result.clone();
    let other = generate_identity();
    impostor.receiver_pubkey = other.public_key_b64.clone();
    impostor.receiver_fingerprint = other.fingerprint.clone();
    let mut padded = result.clone();
    padded.records[0].push('=');

    for bad in [nonce, op, reordered, impostor, padded] {
        assert!(bad.verify(&rpc, &retained, NOW).is_err(), "{bad:?}");
    }
    // A responder cannot return more than k records.
    let three = [
        records.clone(),
        vec![record(&generate_identity(), NOW + 3600)],
    ]
    .concat();
    assert!(QuicRpcResult::new(&rpc, &retained, &receiver, &three).is_err());
}

#[test]
fn invalid_records_are_dropped_individually() {
    let (sender, receiver) = (generate_identity(), generate_identity());
    let rpc = QuicRpc::new(&sender, &receiver.fingerprint, &find(3), &BINDING).unwrap();
    let retained = rpc.transcript(&BINDING).unwrap();
    let good = record(&generate_identity(), NOW + 3600);
    let expiring = record(&generate_identity(), NOW + 100);
    let mut forged = record(&generate_identity(), NOW + 3600);
    forged.seq += 1;
    let result = QuicRpcResult::new(
        &rpc,
        &retained,
        &receiver,
        &[good.clone(), expiring, forged],
    )
    .unwrap();

    assert_eq!(
        result.verify(&rpc, &retained, NOW + 200).unwrap(),
        vec![good]
    );
}

#[test]
fn ping_accepts_only_the_responders_own_record() {
    let (sender, receiver) = (generate_identity(), generate_identity());
    let rpc = QuicRpc::new(&sender, &receiver.fingerprint, &RpcRequest::Ping, &BINDING).unwrap();
    let retained = rpc.transcript(&BINDING).unwrap();
    let own = record(&receiver, NOW + 3600);
    let foreign = record(&generate_identity(), NOW + 3600);

    let result =
        QuicRpcResult::new(&rpc, &retained, &receiver, std::slice::from_ref(&own)).unwrap();
    assert_eq!(result.verify(&rpc, &retained, NOW).unwrap(), vec![own]);
    let result = QuicRpcResult::new(&rpc, &retained, &receiver, &[foreign]).unwrap();
    assert!(result.verify(&rpc, &retained, NOW).unwrap().is_empty());
}

#[test]
fn tcp_rpc_round_trip_is_bound_to_the_challenge() {
    let (sender, receiver) = (generate_identity(), generate_identity());
    let rpc = TcpRpc::new(&sender, &challenge(&receiver), &RpcRequest::Ping).unwrap();
    let retained = rpc.transcript().unwrap();
    verify(&sender.public_key_b64, &retained, &rpc.signature).unwrap();
    assert!(tcp_sender_key(&rpc).is_ok());

    let own = record(&receiver, NOW + 3600);
    let result = TcpRpcResult::new(&rpc, &retained, &receiver, std::slice::from_ref(&own)).unwrap();
    let parsed: TcpRpcResult = parse(&line(&result)).unwrap();
    assert_eq!(parsed.verify(&rpc, &retained, NOW).unwrap(), vec![own]);

    let mut moved = parsed.clone();
    moved.challenge = hex(&[8; 32]);
    assert!(moved.verify(&rpc, &retained, NOW).is_err());
    let mut other_key = rpc.clone();
    other_key.sender_pubkey = generate_identity().public_key_b64.clone();
    assert!(tcp_sender_key(&other_key).is_err());
}
