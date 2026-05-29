#!/usr/bin/env bash
set -e

cat > crates/werewolf-core/src/fang.rs <<'EOF'
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FangState {
    Opening,
    Active,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FangRecord {
    pub id: String,
    pub peer: String,
    pub local: String,
    pub remote: String,
    pub state: FangState,
}
EOF

python3 - <<'PY'
from pathlib import Path

# lib.rs
p = Path("crates/werewolf-core/src/lib.rs")
s = p.read_text()
if "pub mod fang;" not in s:
    s += "\npub mod fang;\n"
p.write_text(s)

# daemon
p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

s = s.replace(
"use werewolf_core::{",
"use werewolf_core::{\n    fang::{FangRecord, FangState},"
)

s = s.replace(
"struct DaemonState {\n    peers: Vec<PeerRecord>,",
"struct DaemonState {\n    peers: Vec<PeerRecord>,\n    fangs: Vec<FangRecord>,"
)

insert = '''
        "fang.open" => {
            let mut st = state.lock().await;

            let peer = req.args["peer"].as_str().unwrap_or("").trim().to_string();
            let local = req.args["local"].as_str().unwrap_or("").trim().to_string();
            let remote = req.args["remote"].as_str().unwrap_or("").trim().to_string();

            if peer.is_empty() || local.is_empty() || remote.is_empty() {
                return ControlResponse::err(req.id, "FANG_INVALID", "peer, local and remote are required");
            }

            if !st.peers.iter().any(|p| p.name == peer) {
                return ControlResponse::err(req.id, "FANG_UNKNOWN_PEER", format!("Unknown peer: {}", peer));
            }

            let fang_id = format!("fang_{}", st.fangs.len() + 1);

            let fang = FangRecord {
                id: fang_id.clone(),
                peer,
                local,
                remote,
                state: FangState::Active,
            };

            st.fangs.push(fang);
            st.status.active_fangs = st.fangs.len();
            st.status.mode = werewolf_core::state::WolfMode::Wolf;

            ControlResponse::ok(req.id, json!({
                "fang_id": fang_id,
                "state": "active"
            }))
        }

        "fang.list" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.fangs))
        }

        "fang.close" => {
            let mut st = state.lock().await;
            let fang_id = req.args["fang_id"].as_str().unwrap_or("").trim().to_string();

            if fang_id.is_empty() {
                return ControlResponse::err(req.id, "FANG_INVALID", "fang_id is required");
            }

            let before = st.fangs.len();
            st.fangs.retain(|f| f.id != fang_id);

            if st.fangs.len() == before {
                return ControlResponse::err(req.id, "FANG_NOT_FOUND", format!("Fang not found: {}", fang_id));
            }

            st.status.active_fangs = st.fangs.len();

            if st.fangs.is_empty() {
                st.status.mode = werewolf_core::state::WolfMode::Human;
            }

            ControlResponse::ok(req.id, json!({
                "status": "closed",
                "active_fangs": st.fangs.len()
            }))
        }
'''

s = s.replace(
'        "pack.add" => {',
insert + '\n\n        "pack.add" => {'
)

p.write_text(s)

# CLI
p = Path("crates/werewolfctl/src/main.rs")
s = p.read_text()

s = s.replace(
"enum Commands {",
'''enum Commands {
    Fang {
        #[command(subcommand)]
        command: FangCommands,
    },'''
)

s = s.replace(
"enum PackCommands {",
'''#[derive(Subcommand)]
enum FangCommands {
    Open {
        peer: String,
        local: String,
        remote: String,
    },
    List,
    Close {
        fang_id: String,
    },
}

enum PackCommands {'''
)

s = s.replace(
"let (cmd, args) = match cli.command {",
'''let (cmd, args) = match cli.command {
        Commands::Fang { command } => match command {
            FangCommands::Open { peer, local, remote } => (
                "fang.open".to_string(),
                json!({
                    "peer": peer,
                    "local": local,
                    "remote": remote
                })
            ),
            FangCommands::List => (
                "fang.list".to_string(),
                json!({})
            ),
            FangCommands::Close { fang_id } => (
                "fang.close".to_string(),
                json!({ "fang_id": fang_id })
            ),
        },'''
)

p.write_text(s)
PY

echo "🦷 Fang control patch applied."
