use crate::{open_fang_from_parts, policy::requested_transport, state::DaemonState};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::{
    io::{self, BufReader},
    net::UnixStream,
    sync::{Mutex, Semaphore},
};
use werewolf_core::{
    protocol::{ControlRequest, ControlResponse},
    state::WolfMode,
};

mod limits;
mod mutation;
mod socket;
pub(crate) use mutation::persist_active;
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
    if limits::mutates(&req.cmd) && state.lock().await.storage_degraded {
        return ControlResponse::err(req.id, "STORAGE_DEGRADED", "operator intervention required");
    }
    if mutation::handles(&req.cmd) {
        return mutation::handle(req, state, home).await;
    }
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
                    "storage_degraded": st.storage_degraded,
                    "inbound_authority": st.inbound_authority.summary(),
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
                    "storage_degraded": st.storage_degraded,
                    "inbound_authority": st.inbound_authority.summary(),
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

        "pack.list" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.peers))
        }

        "fang.open" => {
            let peer = req.args["peer"].as_str().unwrap_or("").trim().to_string();
            let local = req.args["local"].as_str().unwrap_or("").trim().to_string();
            let remote = req.args["remote"].as_str().unwrap_or("").trim().to_string();

            let transport = requested_transport(req.args["transport"].as_str());

            open_fang_from_parts(req.id, state.clone(), peer, local, remote, transport).await
        }

        "fang.profile.list" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.fang_profiles))
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

            crate::open_fang_with_profile(
                req.id,
                state.clone(),
                profile.peer,
                profile.local,
                profile.remote,
                profile.transport,
                Some((home, profile_name)),
            )
            .await
        }

        "silver.trigger" => {
            let authority = state.lock().await.inbound_authority.clone();
            let epoch = match authority.lock() {
                Ok(e) => e,
                Err(_) => {
                    return ControlResponse::err(
                        req.id,
                        "SILVER_FAILED",
                        "local state mutation rejected",
                    )
                }
            };
            let den = {
                let st = state.lock().await;
                st.den.clone()
            };
            let Some(den) = den else {
                return ControlResponse::err(
                    req.id,
                    "SILVER_FAILED",
                    "local state mutation rejected",
                );
            };
            let bytes = br#"{"version":1,"mode":"locked"}"#;
            let outcome = tokio::task::spawn_blocking(move || {
                den.replace(std::ffi::OsStr::new("silver.json"), bytes, false)
            })
            .await;
            if !matches!(
                outcome,
                Ok(werewolf_core::local_fs::CommitOutcome::DurablyCommitted)
            ) {
                return ControlResponse::err(
                    req.id,
                    "SILVER_SAVE_FAILED",
                    "local state mutation rejected",
                );
            }
            let mut st = state.lock().await;
            st.fang_registry.abort_all();
            st.fang_registry.clear_started();

            st.fang_registry.clear_records();
            st.status.active_fangs = 0;
            st.status.mode = WolfMode::Silver;
            st.status.silver = "active".to_string();

            let _ = epoch;
            ControlResponse::ok(
                req.id,
                json!({
                    "status": "silver_active",
                    "message": "Silver mode active. New Fangs rejected."
                }),
            )
        }

        "silver.reset" => {
            let authority = state.lock().await.inbound_authority.clone();
            let expected = authority.epoch().unwrap_or(0);
            let den = state.lock().await.den.clone();
            let Some(den) = den else {
                return ControlResponse::err(
                    req.id,
                    "SILVER_FAILED",
                    "local state mutation rejected",
                );
            };
            let outcome = tokio::task::spawn_blocking(move || {
                den.replace(
                    std::ffi::OsStr::new("silver.json"),
                    br#"{"version":1,"mode":"open"}"#,
                    false,
                )
            })
            .await;
            if !matches!(
                outcome,
                Ok(werewolf_core::local_fs::CommitOutcome::DurablyCommitted)
            ) {
                return ControlResponse::err(
                    req.id,
                    "SILVER_SAVE_FAILED",
                    "local state mutation rejected",
                );
            }
            if authority.open_after_durable(expected).is_err() {
                return ControlResponse::err(
                    req.id,
                    "SILVER_SUPERSEDED",
                    "local state mutation rejected",
                );
            }
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
            let st = state.lock().await;
            let fang_id = req.args["fang_id"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();

            if fang_id.is_empty() {
                return ControlResponse::err(req.id, "FANG_INVALID", "fang_id is required");
            }

            let Some(closed_fang) = st.fang_registry.find_id(&fang_id).cloned() else {
                return ControlResponse::err(req.id, "FANG_NOT_FOUND", "Fang not found");
            };
            let mut candidate = st.active_profiles.clone();
            candidate.retain(|name| {
                !st.fang_profiles.iter().any(|profile| {
                    profile.name == *name
                        && profile.peer == closed_fang.peer
                        && profile.local == closed_fang.local
                        && profile.remote == closed_fang.remote
                })
            });
            let changed = candidate != st.active_profiles;
            drop(st);
            if changed && persist_active(&home, candidate, &state).await.is_err() {
                return ControlResponse::err(req.id, "ACTIVE_SAVE_FAILED", "close rejected");
            }
            let mut st = state.lock().await;
            st.fang_registry.retain_records(|f| f.id != fang_id);

            if let Some(handle) = st.fang_registry.remove_task(&fang_id) {
                handle.abort();
            }

            if let Some(cancellation) = st.fang_registry.remove_cancellation(&fang_id) {
                cancellation.abort_children();
            }

            st.fang_registry.remove_started(&fang_id);

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
    use werewolf_core::pelt::generate_identity;

    #[tokio::test]
    async fn remove_and_revoke_retain_runtime_denial_when_pack_commit_fails() {
        use crate::authority::{PeerAuthority, Transport};
        for command in ["pack.revoke", "pack.remove"] {
            let fixture = Fixture::new();
            let identity = generate_identity();
            let peer = serde_json::from_value(json!({
                "name":"peer", "fingerprint":identity.fingerprint,
                "public_key_b64":identity.public_key_b64,
                "address":"tcp://127.0.0.1:1", "trust":"Packmate"
            }))
            .unwrap();
            let state = Arc::new(Mutex::new(DaemonState {
                peers: vec![peer],
                ..fixture.state()
            }));
            let authority = state.lock().await.inbound_authority.clone();
            let ticket = authority.ticket(&identity.fingerprint).unwrap();
            let lease = authority.reserve(ticket, Transport::Tcp).unwrap();
            lease.publish().unwrap();
            let cancelled = tokio::spawn(async move {
                lease.cancelled().await;
                drop(lease);
            });
            let path = fixture.0.join("pack.json");
            werewolf_core::pack::save_pack(&path, &state.lock().await.peers).unwrap();
            let before = std::fs::read(&path).unwrap();
            // Unsafe destination produces NotCommitted without replacing it.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            let request = || ControlRequest {
                id: "test".into(),
                cmd: command.into(),
                args: json!({"name":"peer"}),
            };
            let response = handle_request(request(), state.clone(), fixture.0.clone()).await;
            assert!(!response.ok);
            assert_eq!(
                response.error.unwrap().code,
                "PACK_SAVE_FAILED_RUNTIME_DENIED"
            );
            tokio::time::timeout(std::time::Duration::from_secs(2), cancelled)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(state.lock().await.peers.len(), 1);
            assert!(!state.lock().await.storage_degraded);
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert_eq!(
                authority.peer_state(&identity.fingerprint, true).unwrap(),
                PeerAuthority::RuntimeDeniedPendingDurability
            );
            assert!(authority.reserve(ticket, Transport::Tcp).is_err());
            assert_eq!(
                authority.summary()["pending_durability"],
                json!([identity.fingerprint])
            );
            // Explicit fixture/operator repair permits a durable retry; no
            // production validation path silently repairs the unsafe mode.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(
                handle_request(request(), state.clone(), fixture.0.clone())
                    .await
                    .ok
            );
            assert!(state.lock().await.peers.is_empty());
            assert!(werewolf_core::pack::load_pack(&path).unwrap().is_empty());
            assert_eq!(
                authority.peer_state(&identity.fingerprint, false).unwrap(),
                PeerAuthority::Revoked
            );
            assert!(authority.reserve(ticket, Transport::Tcp).is_err());
            let committed = std::fs::read(&path).unwrap();
            // A repeated name removal reports absence, with idempotent state:
            // it cannot re-authorize, write another document, or revive a lease.
            assert!(
                !handle_request(request(), state.clone(), fixture.0.clone())
                    .await
                    .ok
            );
            assert_eq!(std::fs::read(&path).unwrap(), committed);
            authority.cleanup(None).await.unwrap();
        }
    }

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
    impl Fixture {
        fn state(&self) -> DaemonState {
            DaemonState {
                den: Some(Arc::new(
                    werewolf_core::local_fs::PrivateDirectory::open(&self.0, false).unwrap(),
                )),
                ..DaemonState::default()
            }
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
            Arc::new(Mutex::new(fixture.state())),
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
        let state = Arc::new(Mutex::new(fixture.state()));
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
            Arc::new(Mutex::new(fixture.state())),
            fixture.0.clone(),
            Arc::new(limits::Mutations::default()),
            tokio::time::Instant::now() + std::time::Duration::from_millis(20),
        )
        .await;
        assert!(result.is_err());
    }
    #[tokio::test]
    async fn failed_pack_save_preserves_live_memory() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("pack.json")).unwrap();
        let state = Arc::new(Mutex::new(fixture.state()));
        let identity = generate_identity();
        let response = handle_request(ControlRequest {
            id: "fixture".into(), cmd: "pack.add".into(),
            args: json!({"name":"fixture", "fingerprint":identity.fingerprint, "address":"tcp://127.0.0.1:1"})
        }, state.clone(), fixture.0.clone()).await;
        assert!(!response.ok);
        assert_eq!(state.lock().await.peers.len(), 0);
        assert!(fixture.0.join("pack.json").is_dir());
    }
    #[tokio::test]
    async fn accepted_mutation_survives_client_task_cancellation() {
        use tokio::io::AsyncWriteExt;
        let fixture = Fixture::new();
        let state = Arc::new(Mutex::new(fixture.state()));
        let guard = state.lock().await;
        let mutations = Arc::new(limits::Mutations::default());
        let (mut client, server) = UnixStream::pair().unwrap();
        let task = tokio::spawn(handle_control_client(
            server,
            state.clone(),
            fixture.0.clone(),
            mutations.clone(),
            tokio::time::Instant::now() + limits::LIFETIME,
        ));
        client
            .write_all(b"{\"id\":\"test\",\"cmd\":\"pelt.init\"}\n")
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while mutations.active_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        drop(client);
        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(identity) = &state.lock().await.pelt {
                    let saved: serde_json::Value = serde_json::from_slice(
                        &std::fs::read(fixture.0.join("pelt.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(saved["fingerprint"], identity.fingerprint);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn mutation_keeps_the_startup_den_descriptor_after_path_replacement() {
        let outer = Fixture::new();
        let original = outer.0.join("den");
        let directory = werewolf_core::local_fs::PrivateDirectory::open(&original, true).unwrap();
        let state = Arc::new(Mutex::new(DaemonState {
            den: Some(Arc::new(directory)),
            ..DaemonState::default()
        }));
        let moved = outer.0.join("moved");
        std::fs::rename(&original, &moved).unwrap();
        let _replacement =
            werewolf_core::local_fs::PrivateDirectory::open(&original, true).unwrap();
        let response = handle_request(
            ControlRequest {
                id: "test".into(),
                cmd: "pelt.init".into(),
                args: json!({}),
            },
            state.clone(),
            original.clone(),
        )
        .await;
        assert!(response.ok);
        assert!(!original.join("pelt.json").exists());
        let saved: werewolf_core::pelt::PeltIdentity =
            serde_json::from_slice(&std::fs::read(moved.join("pelt.json")).unwrap()).unwrap();
        assert_eq!(
            saved.fingerprint,
            state.lock().await.pelt.as_ref().unwrap().fingerprint
        );
    }

    #[tokio::test]
    async fn profile_intent_failure_preserves_registry_and_close_resources() {
        let fixture = Fixture::new();
        let identity = generate_identity();
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = reservation.local_addr().unwrap().to_string();
        drop(reservation);
        let peer = serde_json::from_value(json!({
            "name":"peer", "fingerprint":identity.fingerprint,
            "public_key_b64":identity.public_key_b64,
            "address":"tcp://127.0.0.1:1", "trust":"Packmate"
        }))
        .unwrap();
        let profile = werewolf_core::fang_profile::FangProfile {
            name: "profile".into(),
            peer: "peer".into(),
            local: local.clone(),
            remote: "127.0.0.1:1".into(),
            transport: "tcp-plain".into(),
        };
        let state = Arc::new(Mutex::new(DaemonState {
            pelt: Some(identity),
            peers: vec![peer],
            fang_profiles: vec![profile],
            ..fixture.state()
        }));
        let open = || ControlRequest {
            id: "test".into(),
            cmd: "fang.open_profile".into(),
            args: json!({"name":"profile"}),
        };
        std::fs::create_dir(fixture.0.join("active_fangs.json")).unwrap();
        let rejected = handle_request(open(), state.clone(), fixture.0.clone()).await;
        assert!(!rejected.ok);
        assert!(state.lock().await.fang_registry.is_empty());
        assert!(state.lock().await.active_profiles.is_empty());
        // Failed preparation must release its listener before a retry can bind.
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if tokio::net::TcpListener::bind(&local).await.is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        std::fs::remove_dir(fixture.0.join("active_fangs.json")).unwrap();
        let opened = handle_request(open(), state.clone(), fixture.0.clone()).await;
        assert!(opened.ok);
        let id = state.lock().await.fang_registry.records()[0].id.clone();
        assert_eq!(state.lock().await.active_profiles, vec!["profile"]);
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(
                &std::fs::read(fixture.0.join("active_fangs.json")).unwrap()
            )
            .unwrap(),
            vec!["profile"]
        );
        let close = || ControlRequest {
            id: "test".into(),
            cmd: "fang.close".into(),
            args: json!({"fang_id":id}),
        };
        std::fs::set_permissions(
            fixture.0.join("active_fangs.json"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(
            !handle_request(close(), state.clone(), fixture.0.clone())
                .await
                .ok
        );
        assert_eq!(state.lock().await.fang_registry.len(), 1);
        assert_eq!(state.lock().await.active_profiles, vec!["profile"]);
        std::fs::set_permissions(
            fixture.0.join("active_fangs.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert!(
            handle_request(close(), state.clone(), fixture.0.clone())
                .await
                .ok
        );
        assert!(state.lock().await.fang_registry.is_empty());
        assert!(state.lock().await.active_profiles.is_empty());
        assert!(serde_json::from_slice::<Vec<String>>(
            &std::fs::read(fixture.0.join("active_fangs.json")).unwrap()
        )
        .unwrap()
        .is_empty());
    }

    #[tokio::test]
    async fn concurrent_identity_initialization_has_one_durable_winner() {
        let fixture = Fixture::new();
        let state = Arc::new(Mutex::new(fixture.state()));
        let request = || ControlRequest {
            id: "test".into(),
            cmd: "pelt.init".into(),
            args: json!({}),
        };
        // Bypass admission deliberately: no-replace persistence must protect
        // initial publication even when two candidates are prepared concurrently.
        let (first, second) = tokio::join!(
            handle_request(request(), state.clone(), fixture.0.clone()),
            handle_request(request(), state.clone(), fixture.0.clone()),
        );
        assert_ne!(first.ok, second.ok);
        let bytes = std::fs::read(fixture.0.join("pelt.json")).unwrap();
        let persisted: werewolf_core::pelt::PeltIdentity = serde_json::from_slice(&bytes).unwrap();
        werewolf_core::state_validation::identity(&persisted).unwrap();
        let live = state.lock().await;
        let identity = live.pelt.as_ref().unwrap();
        assert_eq!(persisted.fingerprint, identity.fingerprint);
        assert_eq!(persisted.public_key_b64, identity.public_key_b64);
        assert!(!live.storage_degraded);
    }

    #[tokio::test]
    async fn identity_initialization_never_replaces_existing_state() {
        let fixture = Fixture::new();
        let state = Arc::new(Mutex::new(fixture.state()));
        let request = || ControlRequest {
            id: "test".into(),
            cmd: "pelt.init".into(),
            args: json!({}),
        };
        assert!(
            handle_request(request(), state.clone(), fixture.0.clone())
                .await
                .ok
        );
        let before = std::fs::read(fixture.0.join("pelt.json")).unwrap();
        let response = handle_request(request(), state, fixture.0.clone()).await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "ALREADY_INITIALIZED");
        assert!(std::fs::read(fixture.0.join("pelt.json")).unwrap() == before);
        std::fs::write(fixture.0.join("pelt.json"), b"invalid existing identity").unwrap();
        let state = Arc::new(Mutex::new(fixture.state()));
        assert!(
            !handle_request(request(), state.clone(), fixture.0.clone())
                .await
                .ok
        );
        assert!(state.lock().await.pelt.is_none());
        assert_eq!(
            std::fs::read(fixture.0.join("pelt.json")).unwrap(),
            b"invalid existing identity"
        );
    }
}
