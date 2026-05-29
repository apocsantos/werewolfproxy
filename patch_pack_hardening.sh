#!/usr/bin/env bash
set -e

python3 - <<'PY'
from pathlib import Path

# Patch daemon pack.add and add pack.remove
p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

old = '''        "pack.add" => {
            let mut st = state.lock().await;

            let name = req.args["name"].as_str().unwrap_or("").to_string();
            let fingerprint = req.args["fingerprint"].as_str().unwrap_or("").to_string();
            let address = req.args["address"].as_str().unwrap_or("").to_string();

            let peer = PeerRecord {
                name,
                fingerprint,
                address,
                trust: TrustLevel::Packmate,
            };

            st.peers.push(peer);

            let path = home_pack_path();

            match save_pack(&path, &st.peers) {
                Ok(_) => ControlResponse::ok(req.id, json!({
                    "status": "added",
                    "packmates": st.peers.len()
                })),
                Err(e) => ControlResponse::err(req.id, "PACK_SAVE_FAILED", e.to_string()),
            }
        }

        "pack.list" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.peers))
        }
'''

new = '''        "pack.add" => {
            let mut st = state.lock().await;

            let name = req.args["name"].as_str().unwrap_or("").trim().to_string();
            let fingerprint = req.args["fingerprint"].as_str().unwrap_or("").trim().to_string();
            let address = req.args["address"].as_str().unwrap_or("").trim().to_string();

            if name.is_empty() || fingerprint.is_empty() || address.is_empty() {
                return ControlResponse::err(req.id, "PACK_INVALID", "name, fingerprint and address are required");
            }

            if st.peers.iter().any(|p| p.name == name) {
                return ControlResponse::err(req.id, "PACK_DUP_NAME", format!("Peer name already exists: {}", name));
            }

            if st.peers.iter().any(|p| p.fingerprint == fingerprint) {
                return ControlResponse::err(req.id, "PACK_DUP_FINGERPRINT", format!("Fingerprint already exists: {}", fingerprint));
            }

            if !fingerprint.starts_with("wwp1:") {
                return ControlResponse::err(req.id, "PACK_BAD_FINGERPRINT", "fingerprint must start with wwp1:");
            }

            let peer = PeerRecord {
                name,
                fingerprint,
                address,
                trust: TrustLevel::Packmate,
            };

            st.peers.push(peer);

            let path = home_pack_path();

            match save_pack(&path, &st.peers) {
                Ok(_) => ControlResponse::ok(req.id, json!({
                    "status": "added",
                    "packmates": st.peers.len()
                })),
                Err(e) => ControlResponse::err(req.id, "PACK_SAVE_FAILED", e.to_string()),
            }
        }

        "pack.list" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.peers))
        }

        "pack.remove" => {
            let mut st = state.lock().await;
            let name = req.args["name"].as_str().unwrap_or("").trim().to_string();

            if name.is_empty() {
                return ControlResponse::err(req.id, "PACK_INVALID", "name is required");
            }

            let before = st.peers.len();
            st.peers.retain(|p| p.name != name);

            if st.peers.len() == before {
                return ControlResponse::err(req.id, "PACK_NOT_FOUND", format!("Peer not found: {}", name));
            }

            let path = home_pack_path();

            match save_pack(&path, &st.peers) {
                Ok(_) => ControlResponse::ok(req.id, json!({
                    "status": "removed",
                    "packmates": st.peers.len()
                })),
                Err(e) => ControlResponse::err(req.id, "PACK_SAVE_FAILED", e.to_string()),
            }
        }
'''

if old not in s:
    raise SystemExit("Could not find pack.add block. Stop and show me crates/werewolfd/src/main.rs around pack.add")

s = s.replace(old, new)
p.write_text(s)

# Patch CLI: add Remove
p = Path("crates/werewolfctl/src/main.rs")
s = p.read_text()

s = s.replace(
'''enum PackCommands {
    Add {
        name: String,
        fingerprint: String,
        address: String,
    },
    List,
}''',
'''enum PackCommands {
    Add {
        name: String,
        fingerprint: String,
        address: String,
    },
    Remove {
        name: String,
    },
    List,
}'''
)

s = s.replace(
'''            PackCommands::List => (
                "pack.list".to_string(),
                json!({})
            ),''',
'''            PackCommands::Remove { name } => (
                "pack.remove".to_string(),
                json!({ "name": name })
            ),
            PackCommands::List => (
                "pack.list".to_string(),
                json!({})
            ),'''
)

p.write_text(s)
PY

echo "🐾 Pack hardening patch applied."
