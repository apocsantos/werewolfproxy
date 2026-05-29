#!/usr/bin/env bash
set -e

cat > crates/werewolf-core/src/pack.rs <<'EOF'
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrustLevel {
    Stranger,
    Known,
    Packmate,
    Alpha,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRecord {
    pub name: String,
    pub fingerprint: String,
    pub address: String,
    pub trust: TrustLevel,
}

pub fn load_pack(path: &Path) -> io::Result<Vec<PeerRecord>> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let data = fs::read_to_string(path)?;
    let peers: Vec<PeerRecord> =
        serde_json::from_str(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    Ok(peers)
}

pub fn save_pack(path: &Path, peers: &[PeerRecord]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let data = serde_json::to_string_pretty(peers)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    fs::write(path, data)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}
EOF

python3 - <<'PY'
from pathlib import Path

# update lib.rs
p = Path("crates/werewolf-core/src/lib.rs")
s = p.read_text()

if "pub mod pack;" not in s:
    s += "\npub mod pack;\n"

p.write_text(s)

# patch daemon
p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

s = s.replace(
"use werewolf_core::{",
"use werewolf_core::{\n    pack::{load_pack, save_pack, PeerRecord, TrustLevel},"
)

s = s.replace(
'const PELT_PATH: &str = ".config/werewolf/pelt.json";',
'''const PELT_PATH: &str = ".config/werewolf/pelt.json";
const PACK_PATH: &str = ".config/werewolf/pack.json";'''
)

s = s.replace(
"struct DaemonState {",
'''struct DaemonState {
    peers: Vec<PeerRecord>,'''
)

s = s.replace(
"let state = Arc::new(Mutex::new(initial_state));",
'''let pack_path = home_pack_path();

    match load_pack(&pack_path) {
        Ok(peers) => {
            println!("🐾 Loaded {} packmates", peers.len());
            initial_state.peers = peers;
        }
        Err(e) => {
            eprintln!("⚠️ Failed to load Pack: {}", e);
        }
    }

    let state = Arc::new(Mutex::new(initial_state));'''
)

insert = '''
        "pack.add" => {
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

s = s.replace(
'"pelt.fingerprint" => {',
insert + '\n\n        "pelt.fingerprint" => {'
)

s = s.replace(
"fn home_pelt_path()",
'''fn home_pack_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(PACK_PATH)
}

fn home_pelt_path()'''
)

p.write_text(s)

# patch CLI
p = Path("crates/werewolfctl/src/main.rs")
s = p.read_text()

s = s.replace(
"enum Commands {",
'''enum Commands {
    Pack {
        #[command(subcommand)]
        command: PackCommands,
    },'''
)

s = s.replace(
"enum PeltCommands {",
'''enum PackCommands {
    Add {
        name: String,
        fingerprint: String,
        address: String,
    },
    List,
}

enum PeltCommands {'''
)

s = s.replace(
"let cmd = match cli.command {",
'''let (cmd, args) = match cli.command {
        Commands::Pack { command } => match command {
            PackCommands::Add { name, fingerprint, address } => (
                "pack.add".to_string(),
                json!({
                    "name": name,
                    "fingerprint": fingerprint,
                    "address": address
                })
            ),
            PackCommands::List => (
                "pack.list".to_string(),
                json!({})
            ),
        },'''
)

s = s.replace(
'Commands::Status => "status".to_string(),',
'Commands::Status => ("status".to_string(), json!({})),'
)

s = s.replace(
'PeltCommands::Init => "pelt.init".to_string(),',
'PeltCommands::Init => ("pelt.init".to_string(), json!({})),'
)

s = s.replace(
'PeltCommands::Fingerprint => "pelt.fingerprint".to_string(),',
'PeltCommands::Fingerprint => ("pelt.fingerprint".to_string(), json!({})),'
)

s = s.replace(
"let response = send_request(&cmd).await?;",
"let response = send_request(&cmd, args).await?;"
)

s = s.replace(
"async fn send_request(cmd: &str)",
"async fn send_request(cmd: &str, args: serde_json::Value)"
)

s = s.replace(
"args: json!({}),",
"args,"
)

p.write_text(s)
PY

echo "🐾 Pack patch applied."
