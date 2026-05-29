#!/usr/bin/env bash
set -e

python3 - <<'PY'
from pathlib import Path

p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

# Server side: replace simple OK ACK with signed ACK
old = '''    let mut remote_stream = TcpStream::connect(remote).await?;

    stream.write_all(b"{\\"ok\\":true,\\"session\\":\\"fang-v1\\"}\\n").await?;

    let _ = io::copy_bidirectional(&mut stream, &mut remote_stream).await?;'''

new = '''    let receiver_identity = {
        let st = state.lock().await;
        st.pelt
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "receiver has no Pelt"))?
    };

    let ack_text = format!("fang.ack|{}|{}|{}", receiver_identity.fingerprint, sender_fingerprint, nonce);

    let ack_signature = sign_message(&receiver_identity, ack_text.as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let mut remote_stream = TcpStream::connect(remote).await?;

    let ack = json!({
        "ok": true,
        "session": "fang-v1",
        "receiver_pubkey": receiver_identity.public_key_b64,
        "receiver_fingerprint": receiver_identity.fingerprint,
        "nonce": nonce,
        "signature": ack_signature
    });

    stream.write_all(serde_json::to_string(&ack).unwrap().as_bytes()).await?;
    stream.write_all(b"\\n").await?;

    let _ = io::copy_bidirectional(&mut stream, &mut remote_stream).await?;'''

if old not in s:
    raise SystemExit("Server ACK block not found")

s = s.replace(old, new)

# Client side: replace ACK read with verification
old = '''    let mut reader = BufReader::new(outbound);
    let mut ack = String::new();
    reader.read_line(&mut ack).await?;
    let mut outbound = reader.into_inner();

    let _ = io::copy_bidirectional(inbound, &mut outbound).await?;'''

new = '''    let mut reader = BufReader::new(outbound);
    let mut ack = String::new();
    reader.read_line(&mut ack).await?;
    let mut outbound = reader.into_inner();

    let ack_value: serde_json::Value = serde_json::from_str(&ack)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    if ack_value["ok"].as_bool() != Some(true) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("fang rejected: {}", ack)));
    }

    let receiver_pubkey = ack_value["receiver_pubkey"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing receiver_pubkey"))?;

    let receiver_fingerprint = ack_value["receiver_fingerprint"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing receiver_fingerprint"))?;

    let ack_nonce = ack_value["nonce"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing ack nonce"))?;

    let ack_signature = ack_value["signature"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing ack signature"))?;

    if ack_nonce != nonce {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "ack nonce mismatch"));
    }

    let derived_receiver_fp = fingerprint_from_public_key_b64(receiver_pubkey)
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    if derived_receiver_fp != receiver_fingerprint {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "receiver fingerprint mismatch"));
    }

    let expected_ack_text = format!("fang.ack|{}|{}|{}", receiver_fingerprint, identity.fingerprint, nonce);

    verify_message(receiver_pubkey, expected_ack_text.as_bytes(), ack_signature)
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    let _ = io::copy_bidirectional(inbound, &mut outbound).await?;'''

if old not in s:
    raise SystemExit("Client ACK block not found")

s = s.replace(old, new)

p.write_text(s)
PY

echo "🦷 Mutual Fang Auth v1 patch applied."
