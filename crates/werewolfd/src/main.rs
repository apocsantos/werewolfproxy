mod control;
mod state;
mod transport;
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

mod persistence;
use persistence::{load_active_fang_profiles, load_startup_state};

#[tokio::main]
async fn main() -> anyhow_free::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    ww_info!(
        "SYSTEM",
        "START",
        "🐺 WerewolfProxy {} starting",
        WEREWOLF_VERSION
    );

    let home = expand_home(&args.home);

    let mut initial_state = DaemonState::default();
    configure_den(&mut initial_state, &args, &home);

    load_startup_state(&home, &mut initial_state);

    validate_startup_config(&initial_state);

    let state = Arc::new(Mutex::new(initial_state));

    let active_fangs_path = home.join("active_fangs.json");
    let active_profiles = load_active_fang_profiles(&active_fangs_path);

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

    let listener = control::bind_socket(&args.socket).await?;

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

    if st
        .fangs
        .iter()
        .any(|f| f.local == local && matches!(f.state, FangState::Active))
    {
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

    let peer_addr = match transport.as_str() {
        "tcp" | "tcp-plain" => {
            match parse_fang_transport(&peer_record.address.replace("quic://", "tcp://")) {
                Ok(FangTransport::Tcp(a)) => a,
                _ => {
                    return ControlResponse::err(
                        req_id,
                        "FANG_BAD_TCP_ADDRESS",
                        "Peer TCP address invalid",
                    )
                }
            }
        }

        _ => match parse_fang_transport(&peer_record.address) {
            Ok(FangTransport::Quic(a)) => {
                let fang_id = generate_fang_id(&peer, &local, &remote, st.fangs.len());

                let handle = match open_quic_fang(
                    local.clone(),
                    a.clone(),
                    remote.clone(),
                    identity.clone(),
                )
                .await
                {
                    Ok(h) => h,
                    Err(e) => {
                        return ControlResponse::err(req_id, "QUIC_FANG_FAILED", e.to_string());
                    }
                };

                let fang = FangRecord {
                    id: fang_id.clone(),
                    peer: peer.clone(),
                    local: local.clone(),
                    remote: remote.clone(),
                    state: FangState::Active,
                };

                st.fangs.push(fang);
                st.fang_tasks.insert(fang_id.clone(), handle);
                st.fang_started
                    .insert(fang_id.clone(), std::time::Instant::now());

                st.status.active_fangs = st.fangs.len();
                st.status.mode = WolfMode::Wolf;

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

            _ => {
                return ControlResponse::err(
                    req_id,
                    "FANG_BAD_QUIC_ADDRESS",
                    "Peer QUIC address invalid",
                )
            }
        },
    };

    let fang_id = generate_fang_id(&peer, &local, &remote, st.fangs.len());

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
    let task_transport = transport.clone();

    let handle = tokio::spawn(async move {
        let result = if task_transport == "tcp-plain" {
            transport::run_plain_tcp_forwarder(&task_fang_id, &task_local, &task_remote).await
        } else {
            transport::run_local_fang_forwarder(
                &task_fang_id,
                &task_local,
                &task_peer_addr,
                &task_remote,
                task_identity,
            )
            .await
        };

        if let Err(e) = result {
            eprintln!("fang {} failed: {}", task_fang_id, e);
        }
    });

    st.fang_tasks.insert(fang_id.clone(), handle);
    st.fang_started
        .insert(fang_id.clone(), std::time::Instant::now());

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

#[derive(Debug, Clone)]
enum FangTransport {
    Tcp(String),
    Quic(String),
}

fn parse_fang_transport(address: &str) -> Result<FangTransport, String> {
    if let Some(rest) = address.strip_prefix("tcp://") {
        return Ok(FangTransport::Tcp(rest.to_string()));
    }

    if let Some(rest) = address.strip_prefix("quic://") {
        return Ok(FangTransport::Quic(rest.to_string()));
    }

    Err("peer address must start with tcp:// or quic://".to_string())
}

mod anyhow_free {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
