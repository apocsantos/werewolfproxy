#!/usr/bin/env bash
set -e

python3 - <<'PY'
from pathlib import Path

p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

# Add imports
s = s.replace(
    "use std::{path::PathBuf, sync::Arc};",
    "use std::{collections::HashSet, path::PathBuf, sync::Arc};"
)

# Add nonce cache to DaemonState
s = s.replace(
    '''struct DaemonState {
    peers: Vec<PeerRecord>,
    fangs: Vec<FangRecord>,
    status: Status,
    pelt: Option<PeltIdentity>,
}''',
    '''struct DaemonState {
    peers: Vec<PeerRecord>,
    fangs: Vec<FangRecord>,
    seen_nonces: HashSet<String>,
    status: Status,
    pelt: Option<PeltIdentity>,
}'''
)

# Insert replay check after Pack trust check block
old = '''    {
        let st = state.lock().await;
        if !st.peers.iter().any(|p| p.fingerprint == sender_fingerprint) {
            stream
                .write_all(b"{\\"ok\\":false,\\"error\\":\\"sender not in pack\\"}\\n")
                .await?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "sender not in pack",
            ));
        }
    }

    let receiver_identity = {'''

new = '''    {
        let mut st = state.lock().await;

        if !st.peers.iter().any(|p| p.fingerprint == sender_fingerprint) {
            stream
                .write_all(b"{\\"ok\\":false,\\"error\\":\\"sender not in pack\\"}\\n")
                .await?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "sender not in pack",
            ));
        }

        let replay_key = format!("{}|{}", sender_fingerprint, nonce);

        if st.seen_nonces.contains(&replay_key) {
            stream
                .write_all(b"{\\"ok\\":false,\\"error\\":\\"replay detected\\"}\\n")
                .await?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "replay detected",
            ));
        }

        st.seen_nonces.insert(replay_key);

        if st.seen_nonces.len() > 4096 {
            st.seen_nonces.clear();
        }
    }

    let receiver_identity = {'''

if old not in s:
    raise SystemExit("Could not find Pack trust block. Patch stopped.")

s = s.replace(old, new)

p.write_text(s)
PY

echo "🛡️ Anti-Replay v1 patch applied."
