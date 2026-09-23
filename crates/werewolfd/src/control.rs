use crate::{open_fang_from_parts, policy::requested_transport, state::DaemonState};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::{
    io::{self, BufReader},
    net::UnixStream,
    sync::{oneshot, watch, Mutex, Semaphore},
    task::JoinSet,
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

type MutationTasks = Arc<Mutex<JoinSet<()>>>;

#[cfg(test)]
pub(super) async fn serve(
    listener: socket::ControlListener,
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
) -> io::Result<()> {
    let (keepalive, shutdown) = watch::channel(false);
    let result = serve_until_shutdown(listener, state, home, shutdown).await;
    drop(keepalive);
    result
}

/// Control clients are owned by this supervisor. The shutdown notification
/// stops new accepts, aborts bounded client work, and removes the socket while
/// retaining the per-instance lock. This gives SIGTERM the same local-control
/// cleanup path as an orderly control-server exit.
pub(super) async fn serve_until_shutdown(
    listener: socket::ControlListener,
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    let clients = Arc::new(Semaphore::new(limits::CLIENTS));
    let mutations = Arc::new(limits::Mutations::default());
    let mutation_tasks: MutationTasks = Arc::new(Mutex::new(JoinSet::new()));
    let mut tasks = JoinSet::new();
    let result = loop {
        if let Ok(mut owned) = mutation_tasks.try_lock() {
            while owned.try_join_next().is_some() {}
        }
        let stream = tokio::select! {
            biased;
            Some(_) = tasks.join_next(), if !tasks.is_empty() => continue,
            changed = shutdown.changed() => {
                if changed.is_ok() && *shutdown.borrow() {
                    break Ok(());
                }
                break Err(io::Error::other("control shutdown channel closed"));
            }
            stream = listener.accept() => match stream {
                Ok(stream) => stream,
                Err(error) => break Err(error),
            },
        };
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
        let mutation_tasks = mutation_tasks.clone();

        tasks.spawn(async move {
            let _permit = permit;
            let end = tokio::time::Instant::now() + limits::LIFETIME;
            // Accepted mutations run in an independent task holding their permit;
            // expiration/disconnection of this client cannot cancel publication.
            let _ = tokio::time::timeout_at(
                end,
                handle_control_client(stream, state, home, mutations, mutation_tasks, end),
            )
            .await;
        });
    };
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let mut owned_mutations = mutation_tasks.lock().await;
    owned_mutations.abort_all();
    while owned_mutations.join_next().await.is_some() {}
    drop(owned_mutations);
    listener.close()?;
    result
}

async fn handle_control_client(
    stream: UnixStream,
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
    mutations: Arc<limits::Mutations>,
    mutation_tasks: MutationTasks,
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
            let (complete, response) = oneshot::channel();
            {
                // This lock only owns insertion into the supervisor set; no
                // filesystem, network, or mutation work occurs while held.
                let mut owned = mutation_tasks.lock().await;
                owned.spawn(async move {
                    let _permit = permit;
                    let _ = complete.send(handle_request(req, state, home).await);
                });
            }
            response
                .await
                .map_err(|_| io::Error::other("control operation cancelled"))?
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
            let lifecycle = if matches!(st.status.mode, WolfMode::Silver) {
                "LOCKED"
            } else {
                // Control is started only after persistent-state validation;
                // it is therefore the daemon's observable post-startup state.
                "READY"
            };
            let listener_state = if st.status.pelt_ready {
                "configured"
            } else {
                "waiting_for_pelt"
            };

            ControlResponse::ok(
                req.id,
                json!({
                    "lifecycle": lifecycle,
                    "storage_degraded": st.storage_degraded,
                    "inbound_authority": st.inbound_authority.summary(),
                    "mode": st.status.mode,
                    "pelt_ready": st.status.pelt_ready,
                    "fingerprint": st.pelt.as_ref().map(|pelt| &pelt.fingerprint),
                    "protected_state_generation": st.protected_state_generation,
                    "packmates": st.peers.len(),
                    "fang_profiles": st.fang_profiles.len(),
                    "active_fangs": st.fang_registry.len(),
                    "silver": st.status.silver,
                    "hide": st.status.hide,
                    "listen": st.den_listen,
                    "quic_listen": st.den_quic_listen,
                    "tcp_listener_state": listener_state,
                    "quic_listener_state": listener_state
                }),
            )
        }

        "pelt.fingerprint" => {
            let st = state.lock().await;
            match &st.pelt {
                Some(pelt) => ControlResponse::ok(
                    req.id,
                    json!({
                        "fingerprint": pelt.fingerprint,
                        "public_key_b64": pelt.public_key_b64
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

        "target.list" => {
            let st = state.lock().await;
            let grants: Vec<_> = crate::target_policy::grants_for_display(&st.target_policy)
                .into_iter()
                .map(|(peer, targets)| {
                    json!({
                        "peer_fingerprint": peer,
                        "targets": targets.into_iter().map(|target| target.to_string()).collect::<Vec<_>>()
                    })
                })
                .collect();
            ControlResponse::ok(req.id, json!(grants))
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

        "fang.deactivate_profile" => {
            let name = req.args["name"].as_str().unwrap_or("").trim().to_owned();
            if name.is_empty() {
                return ControlResponse::err(req.id, "FANG_PROFILE_INVALID", "name is required");
            }
            let (home, candidate) = {
                let st = state.lock().await;
                if !st.active_profiles.contains(&name) {
                    return ControlResponse::err(
                        req.id,
                        "FANG_PROFILE_NOT_ACTIVE",
                        "Fang profile is not active",
                    );
                }
                (
                    home.clone(),
                    st.active_profiles
                        .iter()
                        .filter(|profile| *profile != &name)
                        .cloned()
                        .collect::<Vec<_>>(),
                )
            };
            // Persistence is the activation intent boundary. It is committed
            // before cancellation so a restart cannot resurrect the profile.
            if mutation::persist_active(&home, candidate, &state)
                .await
                .is_err()
            {
                return ControlResponse::err(
                    req.id,
                    "ACTIVE_SAVE_FAILED",
                    "activation change rejected",
                );
            }
            let mut st = state.lock().await;
            let ids = st
                .fang_registry
                .records()
                .iter()
                .filter(|fang| {
                    st.fang_profiles.iter().any(|profile| {
                        profile.name == name
                            && profile.peer == fang.peer
                            && profile.local == fang.local
                            && profile.remote == fang.remote
                    })
                })
                .map(|fang| fang.id.clone())
                .collect::<Vec<_>>();
            st.fang_registry.terminate_ids(&ids);
            st.status.active_fangs = st.fang_registry.len();
            if st.fang_registry.is_empty() && !matches!(st.status.mode, WolfMode::Silver) {
                st.status.mode = WolfMode::Human;
            }
            ControlResponse::ok(
                req.id,
                json!({"status":"profile_deactivated","name":name,"closed_fangs":ids.len()}),
            )
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
            if mutation::persist_silver(true, &state).await.is_err() {
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
            if mutation::persist_silver(false, &state).await.is_err() {
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
    use std::{collections::HashMap, os::unix::fs::PermissionsExt};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, oneshot},
        time::timeout,
    };
    use werewolf_core::pelt::generate_identity;

    async fn reserve_tcp_address() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        address
    }

    async fn spawn_receiver(
        receiver: &werewolf_core::pelt::PeltIdentity,
        sender: &werewolf_core::pelt::PeltIdentity,
        target: std::net::SocketAddr,
    ) -> (
        std::net::SocketAddr,
        tokio::task::JoinHandle<io::Result<()>>,
    ) {
        let address = reserve_tcp_address().await;
        let state = Arc::new(Mutex::new(DaemonState {
            inbound_authority: crate::authority::Authority::new(false),
            pelt: Some(receiver.clone()),
            runtime_tls_identity: Some(Arc::new(
                crate::tls_identity::RuntimeTlsIdentity::from_pelt(receiver).unwrap(),
            )),
            peers: vec![werewolf_core::pack::PeerRecord {
                name: "sender".into(),
                fingerprint: sender.fingerprint.clone(),
                public_key_b64: Some(sender.public_key_b64.clone()),
                address: "tcp://127.0.0.1:1".into(),
                trust: werewolf_core::pack::TrustLevel::Packmate,
            }],
            target_policy: crate::target_policy::TargetPolicy::Grants(HashMap::from([(
                sender.fingerprint.clone(),
                [target].into(),
            )])),
            ..DaemonState::default()
        }));
        let (ready_tx, ready_rx) = oneshot::channel();
        let task_state = state;
        let task_address = address.to_string();
        let task = tokio::spawn(async move {
            crate::transport::run_fang_listener_with_ready(&task_address, task_state, ready_tx)
                .await
        });
        ready_rx.await.unwrap().unwrap();
        (address, task)
    }

    async fn spawn_marker_target(
        response: &'static [u8],
    ) -> (
        std::net::SocketAddr,
        mpsc::UnboundedReceiver<Vec<u8>>,
        tokio::task::JoinHandle<io::Result<()>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (seen_tx, seen_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await?;
                let mut request = Vec::new();
                stream.read_to_end(&mut request).await?;
                let _ = seen_tx.send(request);
                stream.write_all(response).await?;
                stream.shutdown().await?;
            }
        });
        (address, seen_rx, task)
    }

    #[derive(Debug, PartialEq, Eq)]
    enum EchoEvent {
        Connected,
        Bytes(Vec<u8>),
        Closed,
    }

    async fn spawn_echo_target() -> (
        std::net::SocketAddr,
        mpsc::UnboundedReceiver<EchoEvent>,
        tokio::task::JoinHandle<io::Result<()>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                let (mut stream, _) = tokio::select! {
                    Some(_) = children.join_next(), if !children.is_empty() => continue,
                    accepted = listener.accept() => accepted?,
                };
                let events = events_tx.clone();
                children.spawn(async move {
                    let _ = events.send(EchoEvent::Connected);
                    let mut buf = [0u8; 1024];
                    loop {
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => {
                                let _ = events.send(EchoEvent::Closed);
                                return;
                            }
                            Ok(n) => {
                                let _ = events.send(EchoEvent::Bytes(buf[..n].to_vec()));
                                if stream.write_all(&buf[..n]).await.is_err() {
                                    let _ = events.send(EchoEvent::Closed);
                                    return;
                                }
                            }
                        }
                    }
                });
            }
        });
        (address, events_rx, task)
    }

    async fn endpoint_roundtrip(
        local: std::net::SocketAddr,
        request: &[u8],
        expected_response: &[u8],
    ) {
        let mut client = TcpStream::connect(local).await.unwrap();
        client.write_all(request).await.unwrap();
        client.shutdown().await.unwrap();
        let mut response = vec![0; expected_response.len()];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(response, expected_response);
        let mut eof = [0u8; 1];
        assert_eq!(client.read(&mut eof).await.unwrap(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pack_address_change_requires_explicit_fang_reactivation() {
        timeout(std::time::Duration::from_secs(15), async {
            let sender = generate_identity();
            let receiver = generate_identity();
            let (target_a, mut target_a_seen, target_a_task) =
                spawn_marker_target(b"ENDPOINT-A").await;
            let (target_b, mut target_b_seen, target_b_task) =
                spawn_marker_target(b"ENDPOINT-B").await;
            let request = b"lc01-request\n";

            let (endpoint_a, endpoint_a_task) = spawn_receiver(&receiver, &sender, target_a).await;
            let (endpoint_b, endpoint_b_task) = spawn_receiver(&receiver, &sender, target_b).await;

            let fixture = Fixture::new();
            let local_a = reserve_tcp_address().await;
            let local_b = reserve_tcp_address().await;
            let state = Arc::new(Mutex::new(DaemonState {
                pelt: Some(sender.clone()),
                peers: vec![werewolf_core::pack::PeerRecord {
                    name: "peer".into(),
                    fingerprint: receiver.fingerprint.clone(),
                    public_key_b64: Some(receiver.public_key_b64.clone()),
                    address: format!("tcp://{endpoint_a}"),
                    trust: werewolf_core::pack::TrustLevel::Packmate,
                }],
                ..fixture.state()
            }));

            let profile_a = handle_request(
                ControlRequest {
                    id: "create-a".into(),
                    cmd: "fang.profile.add".into(),
                    args: json!({
                        "name":"route-a",
                        "peer":"peer",
                        "local":local_a.to_string(),
                        "remote":target_a.to_string(),
                        "transport":"tcp"
                    }),
                },
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(profile_a.ok, "Fang profile creation failed: {profile_a:?}");

            let profile_b = handle_request(
                ControlRequest {
                    id: "create-b".into(),
                    cmd: "fang.profile.add".into(),
                    args: json!({
                        "name":"route-b",
                        "peer":"peer",
                        "local":local_b.to_string(),
                        "remote":target_b.to_string(),
                        "transport":"tcp"
                    }),
                },
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(profile_b.ok, "Fang profile creation failed: {profile_b:?}");

            let opened = handle_request(
                ControlRequest {
                    id: "open-a".into(),
                    cmd: "fang.open_profile".into(),
                    args: json!({"name":"route-a"}),
                },
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(opened.ok, "initial Fang activation failed: {opened:?}");
            endpoint_roundtrip(local_a, request, b"ENDPOINT-A").await;
            assert_eq!(target_a_seen.recv().await.unwrap(), request);

            let updated = handle_request(
                ControlRequest {
                    id: "set-address".into(),
                    cmd: "pack.set_address".into(),
                    args: json!({"name":"peer", "address": format!("quic://{endpoint_b}")}),
                },
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(updated.ok, "Pack address update failed: {updated:?}");
            assert_eq!(updated.result.unwrap()["closed_fangs"], 1);
            assert!(state.lock().await.active_profiles.is_empty());
            assert_eq!(state.lock().await.fang_profiles.len(), 2);

            let active = handle_request(
                ControlRequest {
                    id: "active".into(),
                    cmd: "fang.list".into(),
                    args: json!({}),
                },
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(active.ok);
            assert!(active.result.unwrap().as_array().unwrap().is_empty());

            // A restart reads the persisted Pack/profile documents and must
            // not restore the invalidated active intent.
            let mut restarted = DaemonState::default();
            let restarted_den =
                werewolf_core::local_fs::PrivateDirectory::open(&fixture.0, false).unwrap();
            crate::persistence::load_startup_state(&restarted_den, &mut restarted).unwrap();
            assert!(restarted.active_profiles.is_empty());
            assert_eq!(restarted.fang_profiles.len(), 2);

            // AbortHandle::abort is a cancellation request, so a kernel TCP
            // connect may race with listener teardown. Even if that connect
            // succeeds, it must not create forwarding work to target A after
            // the successful control response has returned.
            if let Ok(mut stale) = TcpStream::connect(local_a).await {
                let _ = stale.write_all(request).await;
                let _ = stale.shutdown().await;
            }

            let reactivated = handle_request(
                ControlRequest {
                    id: "open-b".into(),
                    cmd: "fang.open_profile".into(),
                    args: json!({"name":"route-b"}),
                },
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(reactivated.ok, "reactivation failed: {reactivated:?}");
            endpoint_roundtrip(local_b, request, b"ENDPOINT-B").await;
            assert_eq!(target_b_seen.recv().await.unwrap(), request);
            assert!(target_a_seen.try_recv().is_err());

            let reopened_id = reactivated.result.unwrap()["fang_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let _ = handle_request(
                ControlRequest {
                    id: "close-b".into(),
                    cmd: "fang.close".into(),
                    args: json!({"fang_id": reopened_id}),
                },
                state,
                fixture.0.clone(),
            )
            .await;
            endpoint_a_task.abort();
            endpoint_b_task.abort();
            target_a_task.abort();
            target_b_task.abort();
        })
        .await
        .unwrap();
    }

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

        fn write(&self, name: &str, bytes: &[u8]) {
            let directory =
                werewolf_core::local_fs::PrivateDirectory::open(&self.0, false).unwrap();
            assert!(matches!(
                directory.replace(std::ffi::OsStr::new(name), bytes, false),
                werewolf_core::local_fs::CommitOutcome::DurablyCommitted
            ));
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
    async fn pack_address_change_invalidates_all_matching_fangs_only() {
        let fixture = Fixture::new();
        let peer_a = generate_identity();
        let peer_b = generate_identity();
        let profiles = vec![
            werewolf_core::fang_profile::FangProfile {
                name: "a-one".into(),
                peer: "a".into(),
                local: "127.0.0.1:41001".into(),
                remote: "127.0.0.1:42001".into(),
                transport: "tcp".into(),
            },
            werewolf_core::fang_profile::FangProfile {
                name: "a-two".into(),
                peer: "a".into(),
                local: "127.0.0.1:41002".into(),
                remote: "127.0.0.1:42002".into(),
                transport: "tcp".into(),
            },
            werewolf_core::fang_profile::FangProfile {
                name: "a-inactive".into(),
                peer: "a".into(),
                local: "127.0.0.1:41003".into(),
                remote: "127.0.0.1:42003".into(),
                transport: "tcp".into(),
            },
            werewolf_core::fang_profile::FangProfile {
                name: "b-one".into(),
                peer: "b".into(),
                local: "127.0.0.1:41004".into(),
                remote: "127.0.0.1:42004".into(),
                transport: "tcp".into(),
            },
        ];
        let mut registry = crate::fang_registry::FangRegistry::default();
        for (id, peer) in [("a-one", "a"), ("a-two", "a"), ("b-one", "b")] {
            registry.push(werewolf_core::fang::FangRecord {
                id: id.into(),
                peer: peer.into(),
                local: profiles
                    .iter()
                    .find(|profile| profile.name == id)
                    .unwrap()
                    .local
                    .clone(),
                remote: "127.0.0.1:42000".into(),
                state: werewolf_core::fang::FangState::Active,
            });
        }
        let state = Arc::new(Mutex::new(DaemonState {
            peers: vec![
                werewolf_core::pack::PeerRecord {
                    name: "a".into(),
                    fingerprint: peer_a.fingerprint.clone(),
                    public_key_b64: Some(peer_a.public_key_b64.clone()),
                    address: "tcp://127.0.0.1:43001".into(),
                    trust: werewolf_core::pack::TrustLevel::Packmate,
                },
                werewolf_core::pack::PeerRecord {
                    name: "b".into(),
                    fingerprint: peer_b.fingerprint.clone(),
                    public_key_b64: Some(peer_b.public_key_b64.clone()),
                    address: "tcp://127.0.0.1:43002".into(),
                    trust: werewolf_core::pack::TrustLevel::Packmate,
                },
            ],
            fang_profiles: profiles,
            active_profiles: vec!["a-one".into(), "a-two".into(), "b-one".into()],
            fang_registry: registry,
            ..fixture.state()
        }));

        let response = handle_request(
            ControlRequest {
                id: "set-address".into(),
                cmd: "pack.set_address".into(),
                args: json!({"name":"a", "address":"quic://127.0.0.1:43003"}),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(response.ok);
        assert_eq!(response.result.unwrap()["closed_fangs"], 2);

        let live = state.lock().await;
        assert_eq!(live.active_profiles, ["b-one"]);
        assert_eq!(live.fang_profiles.len(), 4);
        assert_eq!(live.fang_registry.records().len(), 1);
        assert_eq!(live.fang_registry.records()[0].peer, "b");
        assert_eq!(live.peers[0].address, "quic://127.0.0.1:43003");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pack_address_response_waits_for_detached_child_exit() {
        use std::sync::Condvar;
        timeout(std::time::Duration::from_secs(10), async {
            let fixture = Fixture::new();
            let peer = generate_identity();
            let cancellation = crate::fang_registry::FangCancellation::default();
            let (closed_tx, closed_rx) = oneshot::channel();
            cancellation.observe_close(closed_tx);
            let gate = Arc::new((std::sync::Mutex::new(false), Condvar::new()));
            let child_gate = gate.clone();
            let (entered_tx, entered_rx) = oneshot::channel();
            assert!(cancellation.spawn(async move {
                let _ = entered_tx.send(());
                let (lock, ready) = &*child_gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = ready.wait(released).unwrap();
                }
            }));
            entered_rx.await.unwrap();
            let mut registry = crate::fang_registry::FangRegistry::default();
            registry.push(werewolf_core::fang::FangRecord {
                id: "fang".into(),
                peer: "peer".into(),
                local: "127.0.0.1:41010".into(),
                remote: "127.0.0.1:42010".into(),
                state: werewolf_core::fang::FangState::Active,
            });
            registry.insert_cancellation("fang".into(), cancellation);
            let state = Arc::new(Mutex::new(DaemonState {
                peers: vec![werewolf_core::pack::PeerRecord {
                    name: "peer".into(),
                    fingerprint: peer.fingerprint.clone(),
                    public_key_b64: Some(peer.public_key_b64.clone()),
                    address: "tcp://127.0.0.1:43010".into(),
                    trust: werewolf_core::pack::TrustLevel::Packmate,
                }],
                fang_profiles: vec![werewolf_core::fang_profile::FangProfile {
                    name: "route".into(),
                    peer: "peer".into(),
                    local: "127.0.0.1:41010".into(),
                    remote: "127.0.0.1:42010".into(),
                    transport: "tcp".into(),
                }],
                active_profiles: vec!["route".into()],
                fang_registry: registry,
                ..fixture.state()
            }));
            let task_state = state.clone();
            let home = fixture.0.clone();
            let mutation = tokio::spawn(async move {
                handle_request(
                    ControlRequest {
                        id: "set-address".into(),
                        cmd: "pack.set_address".into(),
                        args: json!({"name":"peer", "address":"tcp://127.0.0.1:43011"}),
                    },
                    task_state,
                    home,
                )
                .await
            });
            closed_rx.await.unwrap();
            // Detachment is complete, and the state lock is free while the
            // still-running child prevents the control response.
            assert!(state.lock().await.fang_registry.is_empty());
            assert!(!mutation.is_finished());
            {
                let (lock, ready) = &*gate;
                *lock.lock().unwrap() = true;
                ready.notify_one();
            }
            let response = mutation.await.unwrap();
            assert!(response.ok, "{response:?}");
            assert_eq!(response.result.unwrap()["closed_fangs"], 1);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn pack_address_not_committed_preserves_pack_and_active_state() {
        let sender = generate_identity();
        let receiver = generate_identity();
        let (target, _target_seen, target_task) = spawn_marker_target(b"NOT-COMMITTED").await;
        let (receiver_address, receiver_task) = spawn_receiver(&receiver, &sender, target).await;
        let fixture = Fixture::new();
        let local = reserve_tcp_address().await;
        let peer = receiver.clone();
        let old_pack = vec![werewolf_core::pack::PeerRecord {
            name: "peer".into(),
            fingerprint: peer.fingerprint.clone(),
            public_key_b64: Some(peer.public_key_b64.clone()),
            address: format!("tcp://{receiver_address}"),
            trust: werewolf_core::pack::TrustLevel::Packmate,
        }];
        fixture.write("pack.json", &serde_json::to_vec(&old_pack).unwrap());
        let state = Arc::new(Mutex::new(DaemonState {
            pelt: Some(sender),
            peers: old_pack.clone(),
            ..fixture.state()
        }));

        let profile = handle_request(
            ControlRequest {
                id: "create".into(),
                cmd: "fang.profile.add".into(),
                args: json!({
                    "name":"route",
                    "peer":"peer",
                    "local":local.to_string(),
                    "remote":target.to_string(),
                    "transport":"tcp"
                }),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(profile.ok, "Fang profile creation failed: {profile:?}");

        let opened = handle_request(
            ControlRequest {
                id: "open".into(),
                cmd: "fang.open_profile".into(),
                args: json!({"name":"route"}),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(opened.ok, "Fang activation failed: {opened:?}");
        endpoint_roundtrip(local, b"not-committed-request\n", b"NOT-COMMITTED").await;
        assert_eq!(state.lock().await.fang_registry.records().len(), 1);

        let unchanged = handle_request(
            ControlRequest {
                id: "unchanged".into(),
                cmd: "pack.set_address".into(),
                args: json!({"name":"peer", "address":old_pack[0].address}),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(unchanged.ok);
        assert_eq!(unchanged.result.unwrap()["closed_fangs"], 0);
        assert_eq!(state.lock().await.fang_registry.records().len(), 1);

        std::fs::set_permissions(
            fixture.0.join("pack.json"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();

        let response = handle_request(
            ControlRequest {
                id: "set-address".into(),
                cmd: "pack.set_address".into(),
                args: json!({"name":"peer", "address":"tcp://127.0.0.1:43005"}),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "PACK_SAVE_FAILED");
        assert_eq!(state.lock().await.peers[0].address, old_pack[0].address);
        assert_eq!(state.lock().await.active_profiles, ["route"]);
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(
                &std::fs::read(fixture.0.join("active_fangs.json")).unwrap()
            )
            .unwrap(),
            vec!["route"]
        );
        assert_eq!(state.lock().await.fang_registry.records().len(), 1);
        endpoint_roundtrip(local, b"still-works\n", b"NOT-COMMITTED").await;

        let fang_id = state.lock().await.fang_registry.records()[0].id.clone();
        let closed = handle_request(
            ControlRequest {
                id: "close".into(),
                cmd: "fang.close".into(),
                args: json!({"fang_id": fang_id}),
            },
            state,
            fixture.0.clone(),
        )
        .await;
        assert!(closed.ok, "Fang cleanup failed: {closed:?}");
        receiver_task.abort();
        target_task.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pack_address_change_quiesces_all_children_and_preserves_other_peer() {
        timeout(std::time::Duration::from_secs(20), async {
            let sender = generate_identity();
            let peer_a = generate_identity();
            let peer_b = generate_identity();
            let (target_a, mut a_events, target_a_task) = spawn_echo_target().await;
            let (target_b, mut b_events, target_b_task) = spawn_echo_target().await;
            let (endpoint_a, endpoint_a_task) = spawn_receiver(&peer_a, &sender, target_a).await;
            let (endpoint_b, endpoint_b_task) = spawn_receiver(&peer_b, &sender, target_b).await;
            let fixture = Fixture::new();
            let state = Arc::new(Mutex::new(DaemonState {
                pelt: Some(sender),
                peers: vec![
                    werewolf_core::pack::PeerRecord {
                        name: "a".into(),
                        fingerprint: peer_a.fingerprint.clone(),
                        public_key_b64: Some(peer_a.public_key_b64.clone()),
                        address: format!("tcp://{endpoint_a}"),
                        trust: werewolf_core::pack::TrustLevel::Packmate,
                    },
                    werewolf_core::pack::PeerRecord {
                        name: "b".into(),
                        fingerprint: peer_b.fingerprint.clone(),
                        public_key_b64: Some(peer_b.public_key_b64.clone()),
                        address: format!("tcp://{endpoint_b}"),
                        trust: werewolf_core::pack::TrustLevel::Packmate,
                    },
                ],
                ..fixture.state()
            }));
            let mut locals = Vec::new();
            for (name, peer, target) in [
                ("a-one", "a", target_a),
                ("a-two", "a", target_a),
                ("b-one", "b", target_b),
            ] {
                let local = reserve_tcp_address().await;
                let created = handle_request(
                    ControlRequest {
                        id: format!("create-{name}"),
                        cmd: "fang.profile.add".into(),
                        args: json!({
                            "name":name, "peer":peer, "local":local.to_string(),
                            "remote":target.to_string(), "transport":"tcp"
                        }),
                    },
                    state.clone(),
                    fixture.0.clone(),
                )
                .await;
                assert!(created.ok, "{created:?}");
                let opened = handle_request(
                    ControlRequest {
                        id: format!("open-{name}"),
                        cmd: "fang.open_profile".into(),
                        args: json!({"name":name}),
                    },
                    state.clone(),
                    fixture.0.clone(),
                )
                .await;
                assert!(opened.ok, "{opened:?}");
                locals.push(local);
            }

            // Two established children under each affected Fang, plus one
            // established session for the unrelated peer.
            let mut a_clients = Vec::new();
            for local in [locals[0], locals[0], locals[1], locals[1]] {
                let mut client = TcpStream::connect(local).await.unwrap();
                client.write_all(b"before").await.unwrap();
                let mut echo = [0; 6];
                client.read_exact(&mut echo).await.unwrap();
                assert_eq!(&echo, b"before");
                a_clients.push(client);
            }
            let mut b_client = TcpStream::connect(locals[2]).await.unwrap();
            b_client.write_all(b"before").await.unwrap();
            let mut echo = [0; 6];
            b_client.read_exact(&mut echo).await.unwrap();
            assert_eq!(&echo, b"before");

            let updated = handle_request(
                ControlRequest {
                    id: "set-address".into(),
                    cmd: "pack.set_address".into(),
                    args: json!({"name":"a", "address":"tcp://127.0.0.1:43008"}),
                },
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(updated.ok, "{updated:?}");
            assert_eq!(updated.result.unwrap()["closed_fangs"], 2);
            {
                let live = state.lock().await;
                assert_eq!(live.active_profiles, ["b-one"]);
                assert_eq!(live.fang_registry.records().len(), 1);
                assert_eq!(live.fang_registry.records()[0].peer, "b");
            }
            for mut client in a_clients {
                let _ = client.write_all(b"after").await;
                let mut byte = [0u8; 1];
                match client.read(&mut byte).await {
                    Ok(0) | Err(_) => {}
                    Ok(_) => panic!("invalidated Fang still forwarded a response"),
                }
            }
            let mut closed = 0;
            let mut bytes = 0;
            while closed < 4 {
                match a_events.recv().await.unwrap() {
                    EchoEvent::Connected => {}
                    EchoEvent::Bytes(value) => {
                        assert_eq!(value, b"before");
                        bytes += 1;
                    }
                    EchoEvent::Closed => closed += 1,
                }
            }
            assert_eq!(bytes, 4);
            assert!(a_events.try_recv().is_err());
            b_client.write_all(b"after").await.unwrap();
            let mut echo = [0; 5];
            b_client.read_exact(&mut echo).await.unwrap();
            assert_eq!(&echo, b"after");
            assert_eq!(b_events.recv().await.unwrap(), EchoEvent::Connected);
            assert_eq!(
                b_events.recv().await.unwrap(),
                EchoEvent::Bytes(b"before".to_vec())
            );
            assert_eq!(
                b_events.recv().await.unwrap(),
                EchoEvent::Bytes(b"after".to_vec())
            );

            endpoint_a_task.abort();
            endpoint_b_task.abort();
            target_a_task.abort();
            target_b_task.abort();
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pack_address_indeterminate_quiesces_runtime_fail_closed() {
        timeout(std::time::Duration::from_secs(15), async {
            let sender = generate_identity();
            let receiver = generate_identity();
            let (target, mut events, target_task) = spawn_echo_target().await;
            let (endpoint, receiver_task) = spawn_receiver(&receiver, &sender, target).await;
            let fixture = Fixture::new();
            let local = reserve_tcp_address().await;
            let peer = werewolf_core::pack::PeerRecord {
                name: "peer".into(),
                fingerprint: receiver.fingerprint.clone(),
                public_key_b64: Some(receiver.public_key_b64.clone()),
                address: format!("tcp://{endpoint}"),
                trust: werewolf_core::pack::TrustLevel::Packmate,
            };
            fixture.write(
                "pack.json",
                &serde_json::to_vec(&vec![peer.clone()]).unwrap(),
            );
            let state = Arc::new(Mutex::new(DaemonState {
                pelt: Some(sender),
                peers: vec![peer.clone()],
                ..fixture.state()
            }));
            let created = handle_request(
                ControlRequest {
                    id: "create".into(),
                    cmd: "fang.profile.add".into(),
                    args: json!({
                        "name":"route", "peer":"peer", "local":local.to_string(),
                        "remote":target.to_string(), "transport":"tcp"
                    }),
                },
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(created.ok, "{created:?}");
            let opened = handle_request(
                ControlRequest {
                    id: "open".into(),
                    cmd: "fang.open_profile".into(),
                    args: json!({"name":"route"}),
                },
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(opened.ok, "{opened:?}");
            let mut client = TcpStream::connect(local).await.unwrap();
            client.write_all(b"before").await.unwrap();
            let mut echo = [0; 6];
            client.read_exact(&mut echo).await.unwrap();
            assert_eq!(&echo, b"before");
            state.lock().await.pack_commit_indeterminate = true;

            let changed = handle_request(
                ControlRequest {
                    id: "indeterminate".into(),
                    cmd: "pack.set_address".into(),
                    args: json!({"name":"peer", "address":"tcp://127.0.0.1:43009"}),
                },
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(!changed.ok);
            assert_eq!(changed.error.unwrap().code, "PACK_SAVE_FAILED");
            {
                let live = state.lock().await;
                assert!(live.storage_degraded);
                assert!(live.active_profiles.is_empty());
                assert!(live.fang_registry.is_empty());
                assert_eq!(live.peers[0].address, peer.address);
            }
            // The fault is injected after the real Pack rename. No rollback
            // is attempted; the active intent was persisted fail-closed first.
            assert_eq!(
                werewolf_core::pack::load_pack(&fixture.0.join("pack.json")).unwrap()[0].address,
                "tcp://127.0.0.1:43009"
            );
            assert_eq!(
                serde_json::from_slice::<Vec<String>>(
                    &std::fs::read(fixture.0.join("active_fangs.json")).unwrap()
                )
                .unwrap(),
                Vec::<String>::new()
            );
            let _ = client.write_all(b"after").await;
            let mut byte = [0u8; 1];
            assert!(!matches!(client.read(&mut byte).await, Ok(1)));
            assert_eq!(events.recv().await.unwrap(), EchoEvent::Connected);
            assert_eq!(
                events.recv().await.unwrap(),
                EchoEvent::Bytes(b"before".to_vec())
            );
            assert_eq!(events.recv().await.unwrap(), EchoEvent::Closed);
            assert!(events.try_recv().is_err());
            if let Ok(mut stale) = TcpStream::connect(local).await {
                let _ = stale.write_all(b"after").await;
            }
            assert!(events.try_recv().is_err());
            receiver_task.abort();
            target_task.abort();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn pack_address_change_without_active_fang_reports_zero_closed() {
        let fixture = Fixture::new();
        let peer = generate_identity();
        let state = Arc::new(Mutex::new(DaemonState {
            peers: vec![werewolf_core::pack::PeerRecord {
                name: "peer".into(),
                fingerprint: peer.fingerprint.clone(),
                public_key_b64: Some(peer.public_key_b64.clone()),
                address: "tcp://127.0.0.1:43006".into(),
                trust: werewolf_core::pack::TrustLevel::Packmate,
            }],
            ..fixture.state()
        }));

        let response = handle_request(
            ControlRequest {
                id: "set-address".into(),
                cmd: "pack.set_address".into(),
                args: json!({"name":"peer", "address":"quic://127.0.0.1:43007"}),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(response.ok);
        assert_eq!(response.result.unwrap()["closed_fangs"], 0);
        assert_eq!(
            state.lock().await.peers[0].address,
            "quic://127.0.0.1:43007"
        );
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
    async fn lifecycle_shutdown_removes_only_its_owned_control_socket() {
        use std::os::unix::fs::FileTypeExt;

        let fixture = Fixture::new();
        let path = fixture.0.join("control.sock");
        let listener = bind_socket(path.to_str().unwrap()).await.unwrap();
        let (shutdown, receiver) = watch::channel(false);
        let server = tokio::spawn(serve_until_shutdown(
            listener,
            Arc::new(Mutex::new(fixture.state())),
            fixture.0.clone(),
            receiver,
        ));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if std::fs::symlink_metadata(&path)
                    .map(|metadata| metadata.file_type().is_socket())
                    .unwrap_or(false)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        shutdown.send(true).unwrap();
        server.await.unwrap().unwrap();
        assert!(!path.exists());
        let replacement = bind_socket(path.to_str().unwrap()).await.unwrap();
        drop(replacement);
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
            Arc::new(Mutex::new(JoinSet::new())),
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
            Arc::new(Mutex::new(JoinSet::new())),
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
        let mutation_tasks = Arc::new(Mutex::new(JoinSet::new()));
        let (mut client, server) = UnixStream::pair().unwrap();
        let task = tokio::spawn(handle_control_client(
            server,
            state.clone(),
            fixture.0.clone(),
            mutations.clone(),
            mutation_tasks.clone(),
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
        assert!(live.tls_identity_ready.is_ready());
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
        let runtime_before = state.lock().await.runtime_tls_identity.clone().unwrap();
        assert!(state.lock().await.tls_identity_ready.is_ready());
        let response = handle_request(request(), state.clone(), fixture.0.clone()).await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "ALREADY_INITIALIZED");
        let live = state.lock().await;
        assert!(live.tls_identity_ready.is_ready());
        assert!(Arc::ptr_eq(
            &runtime_before,
            live.runtime_tls_identity.as_ref().unwrap()
        ));
        drop(live);
        assert!(std::fs::read(fixture.0.join("pelt.json")).unwrap() == before);
        std::fs::write(fixture.0.join("pelt.json"), b"invalid existing identity").unwrap();
        let state = Arc::new(Mutex::new(fixture.state()));
        assert!(
            !handle_request(request(), state.clone(), fixture.0.clone())
                .await
                .ok
        );
        assert!(state.lock().await.pelt.is_none());
        assert!(!state.lock().await.tls_identity_ready.is_ready());
        assert_eq!(
            std::fs::read(fixture.0.join("pelt.json")).unwrap(),
            b"invalid existing identity"
        );
    }
}
