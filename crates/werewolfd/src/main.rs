#[cfg(not(target_os = "linux"))]
compile_error!("secure local control currently requires Linux");
mod admission;
mod control;
mod fang_registry;
mod handshake;
mod policy;
mod state;
#[cfg(test)]
mod target_authorization_tests;
mod target_policy;
mod transport;
use fang_registry::FangCancellation;
use state::DaemonState;
mod cli;
use cli::Args;
mod quic_fang;
use crate::quic_fang::open_quic_fang;
mod quic_lab;
use clap::Parser;
use rand_core::{OsRng, RngCore};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use werewolf_core::{
    fang::{FangRecord, FangState},
    protocol::ControlResponse,
    state::WolfMode,
};

const WEREWOLF_VERSION: &str = "v0.1.0-rc1";

macro_rules! ww_info {
    ($subsystem:expr, $event:expr, $($arg:tt)*) => {
        println!("[INFO][{}][{}] {}", $subsystem, $event, format!($($arg)*));
    };
}

macro_rules! ww_warn {
    ($subsystem:expr, $event:expr, $($arg:tt)*) => {
        eprintln!("[WARN][{}][{}] {}", $subsystem, $event, format!($($arg)*));
    };
}

mod config;
use config::{configure_den, expand_home, validate_startup_config};
use policy::{is_plain_tcp, select_peer_transport, ExpectedPeerIdentity, TransportPolicyError};

mod persistence;
use persistence::load_startup_state;

#[tokio::main]
async fn main() -> anyhow_free::Result<()> {
    tracing_subscriber::fmt::init();

    let mut args = Args::parse();
    if args.socket.is_empty() {
        args.socket = werewolf_core::local_fs::default_control_socket()?
            .into_os_string()
            .into_string()
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "non-UTF-8 control path")
            })?;
    }

    ww_info!(
        "SYSTEM",
        "START",
        "🐺 WerewolfProxy {} starting",
        WEREWOLF_VERSION
    );

    let home = expand_home(&args.home);

    let mut initial_state = DaemonState::default();
    configure_den(&mut initial_state, &args, &home);

    let den = werewolf_core::local_fs::PrivateDirectory::open(&home, true)?;
    let _den_lock = den.lock(std::ffi::OsStr::new(".den.lock"))?;
    load_startup_state(&den, &mut initial_state)?;

    validate_startup_config(&initial_state);

    let state = Arc::new(Mutex::new(initial_state));

    let listener = control::bind_socket(&args.socket).await?;

    let active_profiles = state.lock().await.active_profiles.clone();

    if !active_profiles.is_empty() {
        ww_info!(
            "FANG",
            "RESTORE_COUNT",
            "🦷 Restoring {} active Fang profile(s)",
            active_profiles.len()
        );
    }

    for profile_name in active_profiles {
        let profile = {
            let st = state.lock().await;
            st.fang_profiles
                .iter()
                .find(|p| p.name == profile_name)
                .cloned()
        };

        match profile {
            Some(profile) => {
                ww_info!(
                    "FANG",
                    "RESTORE_PROFILE",
                    "🦷 Restoring active Fang profile: {}",
                    profile.name
                );

                let response = open_fang_from_parts(
                    "restore".to_string(),
                    state.clone(),
                    profile.peer,
                    profile.local,
                    profile.remote,
                    profile.transport,
                )
                .await;

                if !response.ok {
                    eprintln!("⚠️ Failed to restore Fang profile: {}", profile_name);
                }
            }
            None => {
                eprintln!("⚠️ Active Fang profile not found: {}", profile_name);
            }
        }
    }

    let net_state = state.clone();
    let listen_addr = args.listen.clone();
    tokio::spawn(async move {
        if let Err(e) = transport::run_fang_listener(&listen_addr, net_state).await {
            eprintln!("🦷 Fang listener error: {}", e);
        }
    });

    let quic_listen_addr = args.quic_listen.clone();
    let quic_state = state.clone();

    tokio::spawn(async move {
        if let Err(e) = transport::run_quic_fang_listener(&quic_listen_addr, quic_state).await {
            eprintln!("⚡ QUIC Fang listener error: {}", e);
        }
    });

    ww_info!(
        "CONTROL",
        "SOCKET_READY",
        "🐺 werewolfd control socket: {}",
        args.socket
    );
    ww_info!(
        "FANG",
        "TCP_LISTENER",
        "🦷 Fang network listener: tcp://{}",
        args.listen
    );
    ww_info!(
        "QUIC",
        "LISTENER",
        "⚡ QUIC Fang listener: quic://{}",
        args.quic_listen
    );
    ww_info!("DEN", "HOME", "🏠 Den home: {}", home.display());

    control::serve(listener, state, home).await?;
    Ok(())
}

fn generate_fang_id(peer: &str, local: &str, remote: &str, existing_count: usize) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let mut random = [0u8; 16];
    OsRng.fill_bytes(&mut random);

    let seed = format!(
        "{}|{}|{}|{}|{}|{:02x?}",
        now, existing_count, peer, local, remote, random
    );

    let hash = blake3::hash(seed.as_bytes());
    let hex = hash.to_hex();

    format!("fang_{}", &hex[..16])
}

async fn open_fang_from_parts(
    req_id: String,
    state: Arc<Mutex<DaemonState>>,
    peer: String,
    local: String,
    remote: String,
    transport: String,
) -> ControlResponse {
    open_fang_with_profile(req_id, state, peer, local, remote, transport, None).await
}

async fn open_fang_with_profile(
    req_id: String,
    state: Arc<Mutex<DaemonState>>,
    peer: String,
    local: String,
    remote: String,
    transport: String,
    activation: Option<(std::path::PathBuf, String)>,
) -> ControlResponse {
    let st = state.lock().await;

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

    if st.fang_registry.has_active_local(&local) {
        return ControlResponse::err(
            req_id,
            "FANG_ALREADY_ACTIVE",
            format!("Fang already active on local address: {}", local),
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
        None => return ControlResponse::err(req_id, "NO_PELT", "No local Pelt identity exists"),
    };
    let expected_peer_identity = ExpectedPeerIdentity {
        fingerprint: peer_record.fingerprint.clone(),
    };

    let peer_addr = match select_peer_transport(&transport, &peer_record.address) {
        Ok(transport::FangTransport::Quic(a)) => {
            let fang_id = generate_fang_id(&peer, &local, &remote, st.fang_registry.len());

            let (cancellation, release) = FangCancellation::prepared();
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let handle = match open_quic_fang(
                local.clone(),
                a.clone(),
                remote.clone(),
                identity.clone(),
                cancellation.clone(),
                ready_tx,
                expected_peer_identity.clone(),
            )
            .await
            {
                Ok(h) => h,
                Err(e) => {
                    return ControlResponse::err(req_id, "QUIC_FANG_FAILED", e.to_string());
                }
            };

            match ready_rx.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    handle.abort();
                    return ControlResponse::err(req_id, "FANG_LISTENER_FAILED", error.to_string());
                }
                Err(_) => {
                    handle.abort();
                    return ControlResponse::err(
                        req_id,
                        "FANG_LISTENER_FAILED",
                        "Fang listener stopped before becoming ready",
                    );
                }
            }

            drop(st);
            if persist_activation(&activation, &state).await.is_err() {
                handle.abort();
                return ControlResponse::err(req_id, "ACTIVE_SAVE_FAILED", "activation rejected");
            }
            let mut st = state.lock().await;

            let fang = FangRecord {
                id: fang_id.clone(),
                peer: peer.clone(),
                local: local.clone(),
                remote: remote.clone(),
                state: FangState::Active,
            };

            st.fang_registry.push(fang);
            st.fang_registry.insert_task(fang_id.clone(), handle);
            st.fang_registry
                .insert_cancellation(fang_id.clone(), cancellation);
            st.fang_registry
                .insert_started(fang_id.clone(), std::time::Instant::now());

            st.status.active_fangs = st.fang_registry.len();
            st.status.mode = WolfMode::Wolf;

            if release.send(true).is_err() {
                st.storage_degraded = true;
                return ControlResponse::err(
                    req_id,
                    "STORAGE_DEGRADED",
                    "activation publication failed",
                );
            }

            return ControlResponse::ok(
                req_id,
                json!({
                    "fang_id": fang_id,
                    "peer": peer,
                    "local": local,
                    "remote": remote,
                    "state": "active",
                    "transport": "quic",
                    "quic_server": a
                }),
            );
        }
        Ok(transport::FangTransport::Tcp(a)) => a,
        Err(TransportPolicyError::BadTcpAddress) => {
            return ControlResponse::err(req_id, "FANG_BAD_TCP_ADDRESS", "Peer TCP address invalid")
        }
        Err(TransportPolicyError::BadQuicAddress) => {
            return ControlResponse::err(
                req_id,
                "FANG_BAD_QUIC_ADDRESS",
                "Peer QUIC address invalid",
            )
        }
    };

    let fang_id = generate_fang_id(&peer, &local, &remote, st.fang_registry.len());

    let fang = FangRecord {
        id: fang_id.clone(),
        peer: peer.clone(),
        local: local.clone(),
        remote: remote.clone(),
        state: FangState::Active,
    };

    let task_fang_id = fang_id.clone();
    let task_local = local.clone();
    let task_peer_addr = peer_addr.clone();
    let task_remote = remote.clone();
    let task_identity = identity.clone();
    let task_transport = transport.clone();
    let (cancellation, release) = FangCancellation::prepared();
    let task_cancellation = cancellation.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let task_expected_peer = expected_peer_identity;

    let handle = tokio::spawn(async move {
        let result = transport::run_selected_forwarder(
            &task_fang_id,
            &task_local,
            &task_peer_addr,
            &task_remote,
            task_identity,
            is_plain_tcp(&task_transport),
            task_cancellation,
            ready_tx,
            task_expected_peer,
        )
        .await;

        if let Err(e) = result {
            eprintln!("fang {} failed: {}", task_fang_id, e);
        }
    });

    match ready_rx.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            handle.abort();
            return ControlResponse::err(req_id, "FANG_LISTENER_FAILED", error.to_string());
        }
        Err(_) => {
            handle.abort();
            return ControlResponse::err(
                req_id,
                "FANG_LISTENER_FAILED",
                "Fang listener stopped before becoming ready",
            );
        }
    }

    drop(st);
    if persist_activation(&activation, &state).await.is_err() {
        handle.abort();
        return ControlResponse::err(req_id, "ACTIVE_SAVE_FAILED", "activation rejected");
    }
    let mut st = state.lock().await;
    st.fang_registry.push(fang);
    st.status.active_fangs = st.fang_registry.len();
    st.status.mode = WolfMode::Wolf;

    st.fang_registry.insert_task(fang_id.clone(), handle);
    st.fang_registry
        .insert_cancellation(fang_id.clone(), cancellation);
    st.fang_registry
        .insert_started(fang_id.clone(), std::time::Instant::now());

    if release.send(true).is_err() {
        st.storage_degraded = true;
        return ControlResponse::err(req_id, "STORAGE_DEGRADED", "activation publication failed");
    }

    ControlResponse::ok(
        req_id,
        json!({
            "fang_id": fang_id,
            "peer": peer,
            "local": local,
            "remote": remote,
            "state": "active",
            "transport": transport,
            "peer_addr": peer_addr
        }),
    )
}

async fn persist_activation(
    activation: &Option<(std::path::PathBuf, String)>,
    state: &Arc<Mutex<DaemonState>>,
) -> std::io::Result<()> {
    if let Some((home, name)) = activation {
        let mut candidate = state.lock().await.active_profiles.clone();
        if !candidate.contains(name) {
            candidate.push(name.clone());
        }
        control::persist_active(home, candidate, state).await?;
    }
    Ok(())
}

mod anyhow_free {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}

#[cfg(test)]
mod replay_v3_tests;
