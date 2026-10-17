#!/usr/bin/env bash
set -e

python3 - <<'PY'
from pathlib import Path

p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

s = s.replace(
    "use std::{collections::{HashMap, HashSet}, path::PathBuf, sync::Arc};",
    "use std::{collections::HashMap, path::PathBuf, sync::Arc, time::{Duration, Instant}};"
)

s = s.replace(
    "seen_nonces: HashSet<String>,",
    "seen_nonces: HashMap<String, Instant>,"
)

old = '''        let replay_key = format!("{}|{}", sender_fingerprint, nonce);

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
        }'''

new = '''        const REPLAY_WINDOW: Duration = Duration::from_secs(300);

        let now = Instant::now();

        st.seen_nonces
            .retain(|_, seen_at| now.duration_since(*seen_at) < REPLAY_WINDOW);

        let replay_key = format!("{}|{}", sender_fingerprint, nonce);

        if st.seen_nonces.contains_key(&replay_key) {
            stream
                .write_all(b"{\\"ok\\":false,\\"error\\":\\"replay detected\\"}\\n")
                .await?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "replay detected",
            ));
        }

        st.seen_nonces.insert(replay_key, now);'''

if old not in s:
    raise SystemExit("Anti-Replay v1 block not found")

s = s.replace(old, new)

p.write_text(s)
PY

echo "🛡️ Anti-Replay v2 patch applied."
