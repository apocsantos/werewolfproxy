use crate::{
    open_fang_from_parts,
    persistence::{forget_active_fang_profile, remember_active_fang_profile},
    state::DaemonState,
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Mutex,
};
use werewolf_core::{
    fang_profile::{save_fang_profiles, FangProfile},
    pack::{save_pack, PeerRecord, TrustLevel},
    pelt::{generate_identity, save_identity},
    protocol::{ControlRequest, ControlResponse},
    state::WolfMode,
};

pub(super) async fn bind_socket(socket: &str) -> io::Result<UnixListener> {
    let _ = fs::remove_file(socket).await;
    let listener = UnixListener::bind(socket)?;
    Ok(listener)
}

pub(super) async fn serve(
    listener: UnixListener,
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
) -> io::Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        let home = home.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_control_client(stream, state, home).await {
                eprintln!("control client error: {}", e);
            }
        });
    }
}

async fn handle_control_client(
    stream: UnixStream,
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
) -> io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<ControlRequest>(&line) {
            Ok(req) => handle_request(req, state.clone(), home.clone()).await,
            Err(e) => ControlResponse::err("unknown", "BAD_JSON", e.to_string()),
        };

        let encoded = serde_json::to_string(&response).unwrap();
        writer.write_all(encoded.as_bytes()).await?;
        writer.write_all(b"\n").await?;
    }

    Ok(())
}

async fn handle_request(
    req: ControlRequest,
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
) -> ControlResponse {
    match req.cmd.as_str() {
        "den.info" => {
            let st = state.lock().await;
            ControlResponse::ok(
                req.id,
                json!({
                    "socket": st.den_socket,
                    "home": st.den_home,
                    "listen": st.den_listen,
                    "quic_listen": st.den_quic_listen,
                    "mode": st.status.mode,
                    "pelt_ready": st.status.pelt_ready,
                    "packmates": st.peers.len(),
                    "fang_profiles": st.fang_profiles.len(),
                    "active_fangs": st.fangs.len(),
                    "silver": st.status.silver,
                    "hide": st.status.hide
                }),
            )
        }

        "status" => {
            let mut st = state.lock().await;
            st.status.packmates = st.peers.len();
            st.status.active_fangs = st.fangs.len();

            ControlResponse::ok(
                req.id,
                json!({
                    "mode": st.status.mode,
                    "pelt_ready": st.status.pelt_ready,
                    "packmates": st.peers.len(),
                    "fang_profiles": st.fang_profiles.len(),
                    "active_fangs": st.fangs.len(),
                    "silver": st.status.silver,
                    "hide": st.status.hide,
                    "listen": st.den_listen,
                    "quic_listen": st.den_quic_listen
                }),
            )
        }

        "pelt.init" => {
            let mut st = state.lock().await;
            let identity = generate_identity();
            let pelt_path = home.join("pelt.json");

            match save_identity(&pelt_path, &identity) {
                Ok(_) => {
                    st.status.pelt_ready = true;
                    st.pelt = Some(identity.clone());
                    ControlResponse::ok(
                        req.id,
                        json!({
                            "fingerprint": identity.fingerprint,
                            "saved_to": pelt_path
                        }),
                    )
                }
                Err(e) => ControlResponse::err(req.id, "PELT_SAVE_FAILED", e.to_string()),
            }
        }

        "pelt.fingerprint" => {
            let st = state.lock().await;
            match &st.pelt {
                Some(pelt) => ControlResponse::ok(
                    req.id,
                    json!({
                        "fingerprint": pelt.fingerprint
                    }),
                ),
                None => ControlResponse::err(
                    req.id,
                    "NO_PELT",
                    "No identity exists. Run pelt.init first.",
                ),
            }
        }

        "pack.add" => {
            let mut st = state.lock().await;

            let name = req.args["name"].as_str().unwrap_or("").trim().to_string();
            let fingerprint = req.args["fingerprint"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            let address = req.args["address"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();

            if name.is_empty() || fingerprint.is_empty() || address.is_empty() {
                return ControlResponse::err(
                    req.id,
                    "PACK_INVALID",
                    "name, fingerprint and address are required",
                );
            }

            if st.peers.iter().any(|p| p.name == name) {
                return ControlResponse::err(
                    req.id,
                    "PACK_DUP_NAME",
                    format!("Peer name already exists: {}", name),
                );
            }

            if st.peers.iter().any(|p| p.fingerprint == fingerprint) {
                return ControlResponse::err(
                    req.id,
                    "PACK_DUP_FINGERPRINT",
                    format!("Fingerprint already exists: {}", fingerprint),
                );
            }

            if !fingerprint.starts_with("wwp1:") {
                return ControlResponse::err(
                    req.id,
                    "PACK_BAD_FINGERPRINT",
                    "fingerprint must start with wwp1:",
                );
            }

            let peer = PeerRecord {
                name,
                fingerprint,
                address,
                trust: TrustLevel::Packmate,
                public_key_b64: None,
            };

            st.peers.push(peer);

            let path = home.join("pack.json");

            match save_pack(&path, &st.peers) {
                Ok(_) => ControlResponse::ok(
                    req.id,
                    json!({
                        "status": "added",
                        "packmates": st.peers.len()
                    }),
                ),
                Err(e) => ControlResponse::err(req.id, "PACK_SAVE_FAILED", e.to_string()),
            }
        }

        "pack.set_address" => {
            let mut st = state.lock().await;

            let name = req.args["name"].as_str().unwrap_or("").trim().to_string();
            let address = req.args["address"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();

            if name.is_empty() || address.is_empty() {
                return ControlResponse::err(
                    req.id,
                    "PACK_INVALID",
                    "name and address are required",
                );
            }

            let peer = match st.peers.iter_mut().find(|p| p.name == name) {
                Some(p) => p,
                None => {
                    return ControlResponse::err(
                        req.id,
                        "PACK_NOT_FOUND",
                        format!("Peer not found: {}", name),
                    )
                }
            };

            peer.address = address.clone();

            let path = home.join("pack.json");

            match save_pack(&path, &st.peers) {
                Ok(_) => ControlResponse::ok(
                    req.id,
                    json!({
                        "status": "address_updated",
                        "name": name,
                        "address": address
                    }),
                ),
                Err(e) => ControlResponse::err(req.id, "PACK_SAVE_FAILED", e.to_string()),
            }
        }

        "pack.list" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.peers))
        }

        "pack.revoke" => {
            let mut st = state.lock().await;

            let name = req.args["name"].as_str().unwrap_or("").trim().to_string();

            if name.is_empty() {
                return ControlResponse::err(req.id, "PACK_INVALID", "name is required");
            }

            let before = st.peers.len();

            let closed_fangs = st.fangs.iter().filter(|f| f.peer == name).count();

            st.fangs.retain(|f| f.peer != name);
            st.fang_tasks.retain(|_, _| true);

            st.peers.retain(|p| p.name != name);

            if st.peers.len() == before {
                return ControlResponse::err(
                    req.id,
                    "PACK_NOT_FOUND",
                    format!("Peer not found: {}", name),
                );
            }

            let path = home.join("pack.json");

            match save_pack(&path, &st.peers) {
                Ok(_) => ControlResponse::ok(
                    req.id,
                    json!({
                        "status": "revoked",
                        "peer": name,
                        "closed_fangs": closed_fangs,
                        "packmates": st.peers.len()
                    }),
                ),
                Err(e) => ControlResponse::err(req.id, "PACK_SAVE_FAILED", e.to_string()),
            }
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
                return ControlResponse::err(
                    req.id,
                    "PACK_NOT_FOUND",
                    format!("Peer not found: {}", name),
                );
            }

            let path = home.join("pack.json");

            match save_pack(&path, &st.peers) {
                Ok(_) => ControlResponse::ok(
                    req.id,
                    json!({
                        "status": "removed",
                        "packmates": st.peers.len()
                    }),
                ),
                Err(e) => ControlResponse::err(req.id, "PACK_SAVE_FAILED", e.to_string()),
            }
        }

        "fang.open" => {
            let peer = req.args["peer"].as_str().unwrap_or("").trim().to_string();
            let local = req.args["local"].as_str().unwrap_or("").trim().to_string();
            let remote = req.args["remote"].as_str().unwrap_or("").trim().to_string();

            let transport = req.args["transport"]
                .as_str()
                .unwrap_or("quic")
                .trim()
                .to_string();

            open_fang_from_parts(req.id, state.clone(), peer, local, remote, transport).await
        }

        "fang.profile.add" => {
            let mut st = state.lock().await;

            let name = req.args["name"].as_str().unwrap_or("").trim().to_string();
            let peer = req.args["peer"].as_str().unwrap_or("").trim().to_string();
            let local = req.args["local"].as_str().unwrap_or("").trim().to_string();
            let remote = req.args["remote"].as_str().unwrap_or("").trim().to_string();

            if name.is_empty() || peer.is_empty() || local.is_empty() || remote.is_empty() {
                return ControlResponse::err(
                    req.id,
                    "FANG_PROFILE_INVALID",
                    "name, peer, local and remote are required",
                );
            }

            if st.fang_profiles.iter().any(|p| p.name == name) {
                return ControlResponse::err(
                    req.id,
                    "FANG_PROFILE_DUP_NAME",
                    format!("Fang profile already exists: {}", name),
                );
            }

            let transport = req.args["transport"]
                .as_str()
                .unwrap_or("quic")
                .trim()
                .to_string();

            st.fang_profiles.push(FangProfile {
                name: name.clone(),
                peer,
                local,
                remote,
                transport,
            });

            let path = home.join("fangs.json");

            match save_fang_profiles(&path, &st.fang_profiles) {
                Ok(_) => ControlResponse::ok(
                    req.id,
                    json!({
                        "status": "profile_added",
                        "name": name,
                        "profiles": st.fang_profiles.len()
                    }),
                ),
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
                return ControlResponse::err(
                    req.id,
                    "FANG_PROFILE_NOT_FOUND",
                    format!("Fang profile not found: {}", name),
                );
            }

            let path = home.join("fangs.json");

            match save_fang_profiles(&path, &st.fang_profiles) {
                Ok(_) => ControlResponse::ok(
                    req.id,
                    json!({
                        "status": "profile_removed",
                        "profiles": st.fang_profiles.len()
                    }),
                ),
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
                    None => {
                        return ControlResponse::err(
                            req.id,
                            "FANG_PROFILE_NOT_FOUND",
                            format!("Fang profile not found: {}", profile_name),
                        )
                    }
                }
            };

            let response = open_fang_from_parts(
                req.id,
                state.clone(),
                profile.peer,
                profile.local,
                profile.remote,
                profile.transport,
            )
            .await;

            if response.ok {
                remember_active_fang_profile(&home, &profile_name);
            }

            response
        }

        "silver.trigger" => {
            let mut st = state.lock().await;

            for (_, handle) in st.fang_tasks.drain() {
                handle.abort();
            }
            st.fang_started.clear();

            st.fangs.clear();
            st.status.active_fangs = 0;
            st.status.mode = WolfMode::Silver;
            st.status.silver = "active".to_string();

            ControlResponse::ok(
                req.id,
                json!({
                    "status": "silver_active",
                    "message": "Silver mode active. New Fangs rejected."
                }),
            )
        }

        "silver.reset" => {
            let mut st = state.lock().await;
            st.status.mode = WolfMode::Human;
            st.status.silver = "armed".to_string();

            ControlResponse::ok(
                req.id,
                json!({
                    "status": "silver_reset",
                    "message": "Silver reset. Wolf returned to Human mode."
                }),
            )
        }

        "fang.cleanup" => {
            let mut st = state.lock().await;

            let dead_ids: Vec<String> = st
                .fang_tasks
                .iter()
                .filter_map(|(id, handle)| {
                    if handle.is_finished() {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
                .collect();

            for id in &dead_ids {
                st.fang_tasks.remove(id);
                st.fang_started.remove(id);
            }

            st.fangs.retain(|f| !dead_ids.contains(&f.id));
            st.status.active_fangs = st.fangs.len();

            ControlResponse::ok(
                req.id,
                json!({
                    "status": "cleanup_done",
                    "removed": dead_ids.len(),
                    "active_fangs": st.fangs.len()
                }),
            )
        }

        "fang.list" => {
            let mut st = state.lock().await;

            let dead_ids: Vec<String> = st
                .fang_tasks
                .iter()
                .filter_map(|(id, handle)| {
                    if handle.is_finished() {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
                .collect();

            for id in &dead_ids {
                st.fang_tasks.remove(id);
                st.fang_started.remove(id);
            }

            st.fangs.retain(|f| !dead_ids.contains(&f.id));
            st.status.active_fangs = st.fangs.len();

            let fangs: Vec<serde_json::Value> = st
                .fangs
                .iter()
                .map(|f| {
                    let transport = st
                        .fang_profiles
                        .iter()
                        .find(|p| p.local == f.local)
                        .map(|p| p.transport.as_str())
                        .unwrap_or("unknown");

                    let uptime_seconds = st
                        .fang_started
                        .get(&f.id)
                        .map(|t| t.elapsed().as_secs())
                        .unwrap_or(0);

                    json!({
                        "id": f.id,
                        "peer": f.peer,
                        "local": f.local,
                        "remote": f.remote,
                        "state": f.state,
                        "transport": transport,
                        "uptime_seconds": uptime_seconds
                    })
                })
                .collect();

            ControlResponse::ok(req.id, json!(fangs))
        }

        "fang.close" => {
            let mut st = state.lock().await;
            let fang_id = req.args["fang_id"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();

            if fang_id.is_empty() {
                return ControlResponse::err(req.id, "FANG_INVALID", "fang_id is required");
            }

            let closed_fang = st.fangs.iter().find(|f| f.id == fang_id).cloned();

            let before = st.fangs.len();
            st.fangs.retain(|f| f.id != fang_id);

            if st.fangs.len() == before {
                return ControlResponse::err(
                    req.id,
                    "FANG_NOT_FOUND",
                    format!("Fang not found: {}", fang_id),
                );
            }

            if let Some(handle) = st.fang_tasks.remove(&fang_id) {
                handle.abort();
            }

            st.fang_started.remove(&fang_id);

            if let Some(fang) = closed_fang {
                if let Some(profile) = st.fang_profiles.iter().find(|p| {
                    p.peer == fang.peer && p.local == fang.local && p.remote == fang.remote
                }) {
                    forget_active_fang_profile(&home, &profile.name);
                    println!("🦷 Forgot active Fang profile: {}", profile.name);
                }
            }

            st.status.active_fangs = st.fangs.len();

            if st.fangs.is_empty() {
                st.status.mode = WolfMode::Human;
            }

            ControlResponse::ok(
                req.id,
                json!({
                    "status": "closed",
                    "active_fangs": st.fangs.len()
                }),
            )
        }

        _ => ControlResponse::err(
            req.id,
            "UNKNOWN_CMD",
            format!("Unknown command: {}", req.cmd),
        ),
    }
}
