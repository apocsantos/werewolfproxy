#!/usr/bin/env bash
set -e

cat > crates/werewolf-core/src/fang_profile.rs <<'EOF'
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FangProfile {
    pub name: String,
    pub peer: String,
    pub local: String,
    pub remote: String,
}

pub fn load_fang_profiles(path: &Path) -> io::Result<Vec<FangProfile>> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let data = fs::read_to_string(path)?;
    let profiles: Vec<FangProfile> =
        serde_json::from_str(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    Ok(profiles)
}

pub fn save_fang_profiles(path: &Path, profiles: &[FangProfile]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let data = serde_json::to_string_pretty(profiles)
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

# core lib
p = Path("crates/werewolf-core/src/lib.rs")
s = p.read_text()
if "pub mod fang_profile;" not in s:
    s += "\npub mod fang_profile;\n"
p.write_text(s)

# daemon
p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

s = s.replace(
    "fang::{FangRecord, FangState},",
    "fang::{FangRecord, FangState},\n    fang_profile::{load_fang_profiles, save_fang_profiles, FangProfile},"
)

s = s.replace(
'''struct DaemonState {
    peers: Vec<PeerRecord>,
    fangs: Vec<FangRecord>,
    fang_tasks: HashMap<String, JoinHandle<()>>,
    seen_nonces: HashMap<String, Instant>,
    status: Status,
    pelt: Option<PeltIdentity>,
}''',
'''struct DaemonState {
    peers: Vec<PeerRecord>,
    fangs: Vec<FangRecord>,
    fang_profiles: Vec<FangProfile>,
    fang_tasks: HashMap<String, JoinHandle<()>>,
    seen_nonces: HashMap<String, Instant>,
    status: Status,
    pelt: Option<PeltIdentity>,
}'''
)

s = s.replace(
'''    let pack_path = home.join("pack.json");
    match load_pack(&pack_path) {
        Ok(peers) => {
            println!("🐾 Loaded {} packmates", peers.len());
            initial_state.peers = peers;
        }
        Err(e) => {
            eprintln!("⚠️ Failed to load Pack: {}", e);
        }
    }''',
'''    let pack_path = home.join("pack.json");
    match load_pack(&pack_path) {
        Ok(peers) => {
            println!("🐾 Loaded {} packmates", peers.len());
            initial_state.peers = peers;
        }
        Err(e) => {
            eprintln!("⚠️ Failed to load Pack: {}", e);
        }
    }

    let fang_profiles_path = home.join("fangs.json");
    match load_fang_profiles(&fang_profiles_path) {
        Ok(profiles) => {
            println!("🦷 Loaded {} Fang profiles", profiles.len());
            initial_state.fang_profiles = profiles;
        }
        Err(e) => {
            eprintln!("⚠️ Failed to load Fang profiles: {}", e);
        }
    }'''
)

# Add helper function before handle_request
marker = "async fn handle_request("
helper = r'''
async fn open_fang_from_parts(
    req_id: String,
    state: Arc<Mutex<DaemonState>>,
    peer: String,
    local: String,
    remote: String,
) -> ControlResponse {
    let mut st = state.lock().await;

    if matches!(st.status.mode, WolfMode::Silver) {
        return ControlResponse::err(
            req_id,
            "SILVER_ACTIVE",
            "Silver mode is active. Fang open rejected.",
        );
    }

    if peer.is_empty() || local.is_empty() || remote.is_empty() {
        return ControlResponse::err(
            req_id,
            "FANG_INVALID",
            "peer, local and remote are required",
        );
    }

    let peer_record = match st.peers.iter().find(|p| p.name == peer) {
        Some(p) => p.clone(),
        None => {
            return ControlResponse::err(
                req_id,
                "FANG_UNKNOWN_PEER",
                format!("Unknown peer: {}", peer),
            )
        }
    };

    let identity = match &st.pelt {
        Some(pelt) => pelt.clone(),
        None => {
            return ControlResponse::err(
                req_id,
                "NO_PELT",
                "No local Pelt identity exists",
            )
        }
    };

    let peer_addr = match parse_tcp_address(&peer_record.address) {
        Ok(a) => a,
        Err(e) => return ControlResponse::err(req_id, "FANG_BAD_PEER_ADDRESS", e),
    };

    let fang_id = format!("fang_{}", st.fangs.len() + 1);

    let fang = FangRecord {
        id: fang_id.clone(),
        peer: peer.clone(),
        local: local.clone(),
        remote: remote.clone(),
        state: FangState::Active,
    };

    st.fangs.push(fang);
    st.status.active_fangs = st.fangs.len();
    st.status.mode = WolfMode::Wolf;

    let task_fang_id = fang_id.clone();
    let task_local = local.clone();
    let task_peer_addr = peer_addr.clone();
    let task_remote = remote.clone();
    let task_identity = identity.clone();

    let handle = tokio::spawn(async move {
        if let Err(e) = run_local_fang_forwarder(
            &task_fang_id,
            &task_local,
            &task_peer_addr,
            &task_remote,
            task_identity,
        )
        .await
        {
            eprintln!("🦷 Fang {} failed: {}", task_fang_id, e);
        }
    });

    st.fang_tasks.insert(fang_id.clone(), handle);

    ControlResponse::ok(
        req_id,
        json!({
            "fang_id": fang_id,
            "state": "active",
            "local": local,
            "peer": peer,
            "remote": remote
        }),
    )
}

'''
if helper.strip() not in s:
    s = s.replace(marker, helper + marker)

# Replace fang.open match block roughly from "fang.open" to before "silver.trigger"
start = s.index('        "fang.open" => {')
end = s.index('        "silver.trigger" => {')
new_block = r'''        "fang.open" => {
            let peer = req.args["peer"].as_str().unwrap_or("").trim().to_string();
            let local = req.args["local"].as_str().unwrap_or("").trim().to_string();
            let remote = req.args["remote"].as_str().unwrap_or("").trim().to_string();

            open_fang_from_parts(req.id, state.clone(), peer, local, remote).await
        }

        "fang.profile.add" => {
            let mut st = state.lock().await;

            let name = req.args["name"].as_str().unwrap_or("").trim().to_string();
            let peer = req.args["peer"].as_str().unwrap_or("").trim().to_string();
            let local = req.args["local"].as_str().unwrap_or("").trim().to_string();
            let remote = req.args["remote"].as_str().unwrap_or("").trim().to_string();

            if name.is_empty() || peer.is_empty() || local.is_empty() || remote.is_empty() {
                return ControlResponse::err(req.id, "FANG_PROFILE_INVALID", "name, peer, local and remote are required");
            }

            if st.fang_profiles.iter().any(|p| p.name == name) {
                return ControlResponse::err(req.id, "FANG_PROFILE_DUP_NAME", format!("Fang profile already exists: {}", name));
            }

            st.fang_profiles.push(FangProfile {
                name: name.clone(),
                peer,
                local,
                remote,
            });

            let path = home.join("fangs.json");

            match save_fang_profiles(&path, &st.fang_profiles) {
                Ok(_) => ControlResponse::ok(req.id, json!({
                    "status": "profile_added",
                    "name": name,
                    "profiles": st.fang_profiles.len()
                })),
                Err(e) => ControlResponse::err(req.id, "FANG_PROFILE_SAVE_FAILED", e.to_string()),
            }
        }

        "fang.profile.list" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.fang_profiles))
        }

        "fang.profile.remove" => {
            let mut st = state.lock().await;
            let name = req.args["name"].as_str().unwrap_or("").trim().to_string();

            if name.is_empty() {
                return ControlResponse::err(req.id, "FANG_PROFILE_INVALID", "name is required");
            }

            let before = st.fang_profiles.len();
            st.fang_profiles.retain(|p| p.name != name);

            if st.fang_profiles.len() == before {
                return ControlResponse::err(req.id, "FANG_PROFILE_NOT_FOUND", format!("Fang profile not found: {}", name));
            }

            let path = home.join("fangs.json");

            match save_fang_profiles(&path, &st.fang_profiles) {
                Ok(_) => ControlResponse::ok(req.id, json!({
                    "status": "profile_removed",
                    "profiles": st.fang_profiles.len()
                })),
                Err(e) => ControlResponse::err(req.id, "FANG_PROFILE_SAVE_FAILED", e.to_string()),
            }
        }

        "fang.open_profile" => {
            let profile_name = req.args["name"].as_str().unwrap_or("").trim().to_string();

            if profile_name.is_empty() {
                return ControlResponse::err(req.id, "FANG_PROFILE_INVALID", "name is required");
            }

            let profile = {
                let st = state.lock().await;
                match st.fang_profiles.iter().find(|p| p.name == profile_name) {
                    Some(p) => p.clone(),
                    None => return ControlResponse::err(req.id, "FANG_PROFILE_NOT_FOUND", format!("Fang profile not found: {}", profile_name)),
                }
            };

            open_fang_from_parts(req.id, state.clone(), profile.peer, profile.local, profile.remote).await
        }

'''
s = s[:start] + new_block + s[end:]

p.write_text(s)

# CLI patch
p = Path("crates/werewolfctl/src/main.rs")
s = p.read_text()

s = s.replace(
'''enum FangCommands {
    Open { peer: String, local: String, remote: String },
    List,
    Close { fang_id: String },
}''',
'''enum FangCommands {
    Open { peer: String, local: String, remote: String },
    OpenProfile { name: String },
    Profile {
        #[command(subcommand)]
        command: FangProfileCommands,
    },
    List,
    Close { fang_id: String },
}

#[derive(Subcommand)]
enum FangProfileCommands {
    Add {
        name: String,
        peer: String,
        local: String,
        remote: String,
    },
    List,
    Remove {
        name: String,
    },
}'''
)

s = s.replace(
'''            FangCommands::List => ("fang.list".to_string(), json!({})),
            FangCommands::Close { fang_id } => (''',
'''            FangCommands::OpenProfile { name } => (
                "fang.open_profile".to_string(),
                json!({ "name": name }),
            ),
            FangCommands::Profile { command } => match command {
                FangProfileCommands::Add { name, peer, local, remote } => (
                    "fang.profile.add".to_string(),
                    json!({
                        "name": name,
                        "peer": peer,
                        "local": local,
                        "remote": remote
                    }),
                ),
                FangProfileCommands::List => (
                    "fang.profile.list".to_string(),
                    json!({}),
                ),
                FangProfileCommands::Remove { name } => (
                    "fang.profile.remove".to_string(),
                    json!({ "name": name }),
                ),
            },
            FangCommands::List => ("fang.list".to_string(), json!({})),
            FangCommands::Close { fang_id } => ('''
)

p.write_text(s)
PY

echo "🦷 Fang profiles v1 patch applied."
