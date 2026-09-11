use crate::{
    open_fang_from_parts,
    persistence::{forget_active_fang_profile, remember_active_fang_profile},
    policy::requested_transport,
    state::DaemonState,
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::{
    io::{self, BufReader},
    net::UnixStream,
    sync::{Mutex, Semaphore},
};
use werewolf_core::{
    fang_profile::{save_fang_profiles, FangProfile},
    pack::{save_pack, PeerRecord, TrustLevel},
    pelt::{generate_identity, save_identity},
    protocol::{ControlRequest, ControlResponse},
    state::WolfMode,
};

mod limits;
mod socket;
pub(crate) use socket::bind as bind_socket;

pub(super) async fn serve(
    listener: socket::ControlListener,
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
) -> io::Result<()> {
    let clients = Arc::new(Semaphore::new(limits::CLIENTS));
    let mutations = Arc::new(limits::Mutations::default());
    loop {
        let stream = listener.accept().await?;
        // No parsing, administrative response or task dispatch before credentials.
        if !socket::authorized(&stream) {
            drop(stream);
            continue;
        }
        let Ok(permit) = clients.clone().try_acquire_owned() else {
            continue;
        };
        let state = state.clone();
        let home = home.clone();
        let mutations = mutations.clone();

        tokio::spawn(async move {
            let _permit = permit;
            let end = tokio::time::Instant::now() + limits::LIFETIME;
            // Accepted mutations run in an independent task holding their permit;
            // expiration/disconnection of this client cannot cancel publication.
            let _ = tokio::time::timeout_at(
                end,
                handle_control_client(stream, state, home, mutations, end),
            )
            .await;
        });
    }
}

async fn handle_control_client(
    stream: UnixStream,
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
    mutations: Arc<limits::Mutations>,
    end: tokio::time::Instant,
) -> io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    for _ in 0..limits::REQUESTS {
        let Some(line) = limits::read_request(&mut reader, end).await? else {
            return Ok(());
        };
        let req = match limits::parse(&line) {
            Ok(req) => req,
            Err(_) => {
                limits::write_response(
                    &mut writer,
                    &ControlResponse::err("unknown", "BAD_REQUEST", "invalid control request"),
                    end,
                )
                .await?;
                return Ok(());
            }
        };
        let response = if limits::mutates(&req.cmd) {
            let permit = mutations.admit().await?;
            let state = state.clone();
            let home = home.clone();
            tokio::spawn(async move {
                let _permit = permit;
                handle_request(req, state, home).await
            })
            .await
            .map_err(|_| io::Error::other("control operation failed"))?
        } else {
            tokio::time::timeout(
                limits::ADMISSION_WINDOW,
                handle_request(req, state.clone(), home.clone()),
            )
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "control operation timeout"))?
        };
        limits::write_response(&mut writer, &response, end).await?;
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
                    "active_fangs": st.fang_registry.len(),
                    "silver": st.status.silver,
                    "hide": st.status.hide
                }),
            )
        }

        "status" => {
            let mut st = state.lock().await;
            st.status.packmates = st.peers.len();
            st.status.active_fangs = st.fang_registry.len();

            ControlResponse::ok(
                req.id,
                json!({
                    "mode": st.status.mode,
                    "pelt_ready": st.status.pelt_ready,
                    "packmates": st.peers.len(),
                    "fang_profiles": st.fang_profiles.len(),
                    "active_fangs": st.fang_registry.len(),
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

            let closed_fangs = st.fang_registry.terminate_peer(&name);

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

            let transport = requested_transport(req.args["transport"].as_str());

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

            let transport = requested_transport(req.args["transport"].as_str());

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

            st.fang_registry.abort_all();
            st.fang_registry.clear_started();

            st.fang_registry.clear_records();
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

            let dead_ids = st.fang_registry.finished_ids();
            st.fang_registry.remove_finished(&dead_ids);
            st.status.active_fangs = st.fang_registry.len();

            ControlResponse::ok(
                req.id,
                json!({
                    "status": "cleanup_done",
                    "removed": dead_ids.len(),
                    "active_fangs": st.fang_registry.len()
                }),
            )
        }

        "fang.list" => {
            let mut st = state.lock().await;

            let dead_ids = st.fang_registry.finished_ids();
            st.fang_registry.remove_finished(&dead_ids);
            st.status.active_fangs = st.fang_registry.len();

            let fangs: Vec<serde_json::Value> = st
                .fang_registry
                .records()
                .iter()
                .map(|f| {
                    let transport = st
                        .fang_profiles
                        .iter()
                        .find(|p| p.local == f.local)
                        .map(|p| p.transport.as_str())
                        .unwrap_or("unknown");

                    let uptime_seconds = st
                        .fang_registry
                        .started_at(&f.id)
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

            let closed_fang = st.fang_registry.find_id(&fang_id).cloned();

            let before = st.fang_registry.len();
            st.fang_registry.retain_records(|f| f.id != fang_id);

            if st.fang_registry.len() == before {
                return ControlResponse::err(
                    req.id,
                    "FANG_NOT_FOUND",
                    format!("Fang not found: {}", fang_id),
                );
            }

            if let Some(handle) = st.fang_registry.remove_task(&fang_id) {
                handle.abort();
            }

            if let Some(cancellation) = st.fang_registry.remove_cancellation(&fang_id) {
                cancellation.abort_children();
            }

            st.fang_registry.remove_started(&fang_id);

            if let Some(fang) = closed_fang {
                if let Some(profile) = st.fang_profiles.iter().find(|p| {
                    p.peer == fang.peer && p.local == fang.local && p.remote == fang.remote
                }) {
                    forget_active_fang_profile(&home, &profile.name);
                    println!("🦷 Forgot active Fang profile: {}", profile.name);
                }
            }

            st.status.active_fangs = st.fang_registry.len();

            if st.fang_registry.is_empty() {
                st.status.mode = WolfMode::Human;
            }

            ControlResponse::ok(
                req.id,
                json!({
                    "status": "closed",
                    "active_fangs": st.fang_registry.len()
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

#[cfg(test)]
mod characterization_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "wwp-control-audit-{}-{}",
                std::process::id(),
                crate::handshake::hex(&crate::handshake::random::<16>().unwrap())
            ));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn existing_regular_file_is_preserved() {
        let fixture = Fixture::new();
        let path = fixture.0.join("control.sock");
        std::fs::write(&path, b"isolated collision victim").unwrap();
        assert!(bind_socket(path.to_str().unwrap()).await.is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"isolated collision victim");
    }

    #[tokio::test]
    async fn active_socket_is_preserved_and_same_uid_authorized() {
        let fixture = Fixture::new();
        let path = fixture.0.join("control.sock");
        let first = bind_socket(path.to_str().unwrap()).await.unwrap();
        assert!(bind_socket(path.to_str().unwrap()).await.is_err());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o600
        );
        let client = UnixStream::connect(&path).await.unwrap();
        let accepted = first.accept().await.unwrap();
        assert!(socket::authorized(&accepted));
        drop((first, client, accepted));
    }

    #[tokio::test]
    async fn stale_socket_is_replaced_but_unlocked_active_socket_is_preserved() {
        use std::os::unix::fs::MetadataExt;
        let fixture = Fixture::new();
        let path = fixture.0.join("control.sock");
        let old = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let inode = std::fs::metadata(&path).unwrap().ino();
        assert!(bind_socket(path.to_str().unwrap()).await.is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
        drop(old);
        let new = bind_socket(path.to_str().unwrap()).await.unwrap();
        let client = UnixStream::connect(&path).await.unwrap();
        let accepted = new.accept().await.unwrap();
        assert!(socket::authorized(&accepted));
        drop((new, client, accepted));
    }

    #[tokio::test]
    async fn unsafe_runtime_and_symlink_fail_before_listener_creation() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let victim = fixture.0.join("victim");
        std::fs::write(&victim, b"untouched").unwrap();
        let path = fixture.0.join("control.sock");
        symlink(&victim, &path).unwrap();
        assert!(bind_socket(path.to_str().unwrap()).await.is_err());
        assert!(std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&victim).unwrap(), b"untouched");
        std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(bind_socket(fixture.0.join("other.sock").to_str().unwrap())
            .await
            .is_err());
        assert!(!fixture.0.join("other.sock").exists());
    }
    #[tokio::test]
    async fn live_control_client_quota_and_release() {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
        let fixture = Fixture::new();
        let path = fixture.0.join("control.sock");
        let listener = bind_socket(path.to_str().unwrap()).await.unwrap();
        let server = tokio::spawn(serve(
            listener,
            Arc::new(Mutex::new(DaemonState::default())),
            fixture.0.clone(),
        ));
        let mut clients = Vec::new();
        for _ in 0..limits::CLIENTS {
            let stream = UnixStream::connect(&path).await.unwrap();
            let mut stream = BufReader::new(stream);
            stream
                .get_mut()
                .write_all(b"{\"id\":\"1\",\"cmd\":\"status\",\"args\":{}}\n")
                .await
                .unwrap();
            let mut response = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                stream.read_line(&mut response),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                serde_json::from_str::<serde_json::Value>(&response).unwrap()["ok"]
                    .as_bool()
                    .unwrap()
            );
            clients.push(stream);
        }
        let mut excess = UnixStream::connect(&path).await.unwrap();
        let mut byte = [0u8];
        let read = tokio::time::timeout(std::time::Duration::from_secs(1), excess.read(&mut byte))
            .await
            .unwrap();
        assert!(matches!(read, Ok(0)) || read.is_err());
        drop(clients.pop());
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let mut replacement = BufReader::new(UnixStream::connect(&path).await.unwrap());
        replacement
            .get_mut()
            .write_all(b"{\"id\":\"1\",\"cmd\":\"status\"}\n")
            .await
            .unwrap();
        let mut response = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            replacement.read_line(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.contains("true"));
        drop(clients);
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn control_request_count_and_lifetime_bounds() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let fixture = Fixture::new();
        let (client, server) = UnixStream::pair().unwrap();
        let state = Arc::new(Mutex::new(DaemonState::default()));
        let home = fixture.0.clone();
        let task = tokio::spawn(handle_control_client(
            server,
            state,
            home,
            Arc::new(limits::Mutations::default()),
            tokio::time::Instant::now() + limits::LIFETIME,
        ));
        let mut client = BufReader::new(client);
        for _ in 0..limits::REQUESTS {
            client
                .get_mut()
                .write_all(b"{\"id\":\"1\",\"cmd\":\"status\"}\n")
                .await
                .unwrap();
            let mut response = String::new();
            client.read_line(&mut response).await.unwrap();
            assert!(response.contains("true"));
        }
        task.await.unwrap().unwrap();
        let mut end = String::new();
        assert_eq!(client.read_line(&mut end).await.unwrap(), 0);
        let (_client, server) = UnixStream::pair().unwrap();
        let result = handle_control_client(
            server,
            Arc::new(Mutex::new(DaemonState::default())),
            fixture.0.clone(),
            Arc::new(limits::Mutations::default()),
            tokio::time::Instant::now() + std::time::Duration::from_millis(20),
        )
        .await;
        assert!(result.is_err());
    }
    #[tokio::test]
    async fn failed_pack_save_currently_leaves_memory_mutated() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("pack.json")).unwrap();
        let state = Arc::new(Mutex::new(DaemonState::default()));
        let identity = generate_identity();
        let response = handle_request(ControlRequest {
            id: "fixture".into(), cmd: "pack.add".into(),
            args: json!({"name":"fixture", "fingerprint":identity.fingerprint, "address":"tcp://127.0.0.1:1"})
        }, state.clone(), fixture.0.clone()).await;
        assert!(!response.ok);
        assert_eq!(state.lock().await.peers.len(), 1);
        assert!(fixture.0.join("pack.json").is_dir());
    }
}
