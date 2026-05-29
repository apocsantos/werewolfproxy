#!/usr/bin/env bash
set -e

python3 - <<'PY'
from pathlib import Path

p = Path("crates/werewolf-core/src/pelt.rs")
s = p.read_text()

s = s.replace(
'use ed25519_dalek::SigningKey;',
'use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};'
)

insert = r'''
pub fn sign_message(identity: &PeltIdentity, message: &[u8]) -> Result<String, String> {
    let secret_bytes = STANDARD
        .decode(&identity.secret_key_b64)
        .map_err(|e| e.to_string())?;

    let secret: [u8; 32] = secret_bytes
        .try_into()
        .map_err(|_| "invalid secret key length".to_string())?;

    let signing_key = SigningKey::from_bytes(&secret);
    let sig = signing_key.sign(message);

    Ok(STANDARD.encode(sig.to_bytes()))
}

pub fn verify_message(public_key_b64: &str, message: &[u8], signature_b64: &str) -> Result<(), String> {
    let public_bytes = STANDARD
        .decode(public_key_b64)
        .map_err(|e| e.to_string())?;

    let public: [u8; 32] = public_bytes
        .try_into()
        .map_err(|_| "invalid public key length".to_string())?;

    let verifying_key = VerifyingKey::from_bytes(&public)
        .map_err(|e| e.to_string())?;

    let sig_bytes = STANDARD
        .decode(signature_b64)
        .map_err(|e| e.to_string())?;

    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| "invalid signature length".to_string())?;

    let sig = Signature::from_bytes(&sig_arr);

    verifying_key
        .verify(message, &sig)
        .map_err(|e| e.to_string())
}

pub fn fingerprint_from_public_key_b64(public_key_b64: &str) -> Result<String, String> {
    let public = STANDARD
        .decode(public_key_b64)
        .map_err(|e| e.to_string())?;

    let mut hasher = Hasher::new();
    hasher.update(&public);
    let hash = hasher.finalize();

    let fp_hex = hexish(&hash.as_bytes()[0..8]);
    Ok(format!("wwp1:{}", fp_hex))
}
'''

if "pub fn sign_message" not in s:
    s = s.replace("fn hexish(bytes: &[u8]) -> String {", insert + "\nfn hexish(bytes: &[u8]) -> String {")

p.write_text(s)
PY

python3 - <<'PY'
from pathlib import Path

p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

s = s.replace(
'pelt::{generate_identity, load_identity, save_identity, PeltIdentity},',
'pelt::{fingerprint_from_public_key_b64, generate_identity, load_identity, save_identity, sign_message, verify_message, PeltIdentity},'
)

# Replace handle_fang_pipe
start = s.index("async fn handle_fang_pipe")
end = s.index("async fn handle_control_client")
new_func = r'''
async fn handle_fang_pipe(stream: TcpStream, state: Arc<Mutex<DaemonState>>) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    reader.read_line(&mut line).await?;
    let mut stream = reader.into_inner();

    let value: serde_json::Value = serde_json::from_str(&line)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let remote = value["remote"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing remote"))?;

    let sender_pubkey = value["sender_pubkey"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing sender_pubkey"))?;

    let sender_fingerprint = value["sender_fingerprint"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing sender_fingerprint"))?;

    let nonce = value["nonce"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing nonce"))?;

    let signature = value["signature"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing signature"))?;

    let derived_fp = fingerprint_from_public_key_b64(sender_pubkey)
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    if derived_fp != sender_fingerprint {
        stream.write_all(b"{\"ok\":false,\"error\":\"fingerprint mismatch\"}\n").await?;
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "fingerprint mismatch"));
    }

    let signed_text = format!("fang.pipe|{}|{}|{}", sender_fingerprint, remote, nonce);

    if let Err(e) = verify_message(sender_pubkey, signed_text.as_bytes(), signature) {
        stream.write_all(b"{\"ok\":false,\"error\":\"bad signature\"}\n").await?;
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, e));
    }

    {
        let st = state.lock().await;
        if !st.peers.iter().any(|p| p.fingerprint == sender_fingerprint) {
            stream.write_all(b"{\"ok\":false,\"error\":\"sender not in pack\"}\n").await?;
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "sender not in pack"));
        }
    }

    let mut remote_stream = TcpStream::connect(remote).await?;

    stream.write_all(b"{\"ok\":true,\"session\":\"fang-v1\"}\n").await?;

    let _ = io::copy_bidirectional(&mut stream, &mut remote_stream).await?;

    Ok(())
}

'''
s = s[:start] + new_func + s[end:]

# Update listener call
s = s.replace(
'if let Err(e) = handle_fang_pipe(stream).await {',
'if let Err(e) = handle_fang_pipe(stream, net_state.clone()).await {'
)

# Patch pipe_one_fang_connection signature and usage
s = s.replace(
'''async fn pipe_one_fang_connection(
    inbound: &mut TcpStream,
    peer_addr: &str,
    remote: &str,
) -> io::Result<()> {''',
'''async fn pipe_one_fang_connection(
    inbound: &mut TcpStream,
    peer_addr: &str,
    remote: &str,
    identity: PeltIdentity,
) -> io::Result<()> {'''
)

s = s.replace(
'''    let hello = json!({
        "cmd": "fang.pipe",
        "remote": remote
    });''',
'''    let nonce = format!("{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos());

    let signed_text = format!("fang.pipe|{}|{}|{}", identity.fingerprint, remote, nonce);

    let signature = sign_message(&identity, signed_text.as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let hello = json!({
        "cmd": "fang.pipe",
        "remote": remote,
        "sender_pubkey": identity.public_key_b64,
        "sender_fingerprint": identity.fingerprint,
        "nonce": nonce,
        "signature": signature
    });'''
)

# Clone identity in fang.open before spawn
s = s.replace(
'''            let peer_record = match st.peers.iter().find(|p| p.name == peer) {
                Some(p) => p.clone(),
                None => return ControlResponse::err(req.id, "FANG_UNKNOWN_PEER", format!("Unknown peer: {}", peer)),
            };''',
'''            let peer_record = match st.peers.iter().find(|p| p.name == peer) {
                Some(p) => p.clone(),
                None => return ControlResponse::err(req.id, "FANG_UNKNOWN_PEER", format!("Unknown peer: {}", peer)),
            };

            let identity = match &st.pelt {
                Some(pelt) => pelt.clone(),
                None => return ControlResponse::err(req.id, "NO_PELT", "No local Pelt identity exists"),
            };'''
)

s = s.replace(
'''            let task_remote = remote.clone();''',
'''            let task_remote = remote.clone();
            let task_identity = identity.clone();'''
)

s = s.replace(
'''                if let Err(e) = run_local_fang_forwarder(&task_fang_id, &task_local, &task_peer_addr, &task_remote).await {''',
'''                if let Err(e) = run_local_fang_forwarder(&task_fang_id, &task_local, &task_peer_addr, &task_remote, task_identity).await {'''
)

# Update run_local_fang_forwarder signature
s = s.replace(
'''async fn run_local_fang_forwarder(
    fang_id: &str,
    local: &str,
    peer_addr: &str,
    remote: &str,
) -> io::Result<()> {''',
'''async fn run_local_fang_forwarder(
    fang_id: &str,
    local: &str,
    peer_addr: &str,
    remote: &str,
    identity: PeltIdentity,
) -> io::Result<()> {'''
)

s = s.replace(
'''        let remote = remote.to_string();
        let fang_id = fang_id.to_string();''',
'''        let remote = remote.to_string();
        let fang_id = fang_id.to_string();
        let identity = identity.clone();'''
)

s = s.replace(
'''            if let Err(e) = pipe_one_fang_connection(&mut inbound, &peer_addr, &remote).await {''',
'''            if let Err(e) = pipe_one_fang_connection(&mut inbound, &peer_addr, &remote, identity).await {'''
)

p.write_text(s)
PY

echo "🦷 Fang Session Layer v1 patch applied."
