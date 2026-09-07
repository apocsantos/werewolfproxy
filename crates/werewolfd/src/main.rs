mod control;
mod state;
mod transport;
use state::DaemonState;
mod cli;
use cli::Args;
mod quic_fang;
use crate::quic_fang::open_quic_fang;
use crate::quic_lab::make_server_endpoint;
mod quic_lab;
use clap::Parser;
use rand_core::{OsRng, RngCore};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::{io, sync::Mutex};
use werewolf_core::{
    fang::{FangRecord, FangState},
    pelt::verify_message,
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

macro_rules! ww_error {
    ($subsystem:expr, $event:expr, $($arg:tt)*) => {
        eprintln!("[ERROR][{}][{}] {}", $subsystem, $event, format!($($arg)*));
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
        if let Err(e) = run_quic_fang_listener(&quic_listen_addr, quic_state).await {
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

async fn run_quic_fang_listener(
    listen_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    let addr: std::net::SocketAddr = listen_addr.parse().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("bad QUIC listen addr: {}", e),
        )
    })?;

    let endpoint = make_server_endpoint(addr)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    ww_info!(
        "QUIC",
        "NATIVE_LISTENER",
        "⚡ Native QUIC Fang listening on {}",
        listen_addr
    );

    while let Some(incoming) = endpoint.accept().await {
        let state = state.clone();

        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => {
                    println!(
                        "⚡ Native QUIC Fang connection from {}",
                        connection.remote_address()
                    );

                    while let Ok((mut send, mut recv)) = connection.accept_bi().await {
                        let state = state.clone();

                        println!("⚡ Native QUIC Fang stream accepted");

                        let mut line_buf = Vec::new();

                        loop {
                            match recv.read_chunk(1, true).await {
                                Ok(Some(chunk)) => {
                                    line_buf.extend_from_slice(&chunk.bytes);

                                    if line_buf.ends_with(b"\n") {
                                        break;
                                    }

                                    if line_buf.len() > 2048 {
                                        eprintln!("QUIC Fang request line too long");
                                        break;
                                    }
                                }
                                Ok(None) => {
                                    eprintln!("stream closed before request line");
                                    break;
                                }
                                Err(e) => {
                                    eprintln!("QUIC request read failed: {}", e);
                                    break;
                                }
                            }
                        }

                        let request_line = String::from_utf8_lossy(&line_buf).trim().to_string();

                        let request_value: serde_json::Value =
                            match serde_json::from_str(&request_line) {
                                Ok(v) => v,
                                Err(e) => {
                                    eprintln!("bad QUIC Fang request JSON: {}", e);
                                    eprintln!("raw request line: {}", request_line);
                                    continue;
                                }
                            };

                        if request_value["cmd"].as_str() != Some("fang.quic.open") {
                            eprintln!("bad QUIC Fang request cmd");
                            continue;
                        }

                        let sender_fp = match request_value["sender_fingerprint"].as_str() {
                            Some(v) => v,
                            None => {
                                eprintln!("missing sender fingerprint");
                                continue;
                            }
                        };

                        let target = match request_value["remote"].as_str() {
                            Some(v) if !v.trim().is_empty() => v.trim().to_string(),
                            _ => {
                                eprintln!("missing QUIC Fang remote target");
                                continue;
                            }
                        };

                        println!("🐾 QUIC sender fingerprint: {}", sender_fp);

                        let sender_public_key = {
                            let st = state.lock().await;

                            match st.peers.iter().find(|p| p.fingerprint == sender_fp) {
                                Some(peer) => match &peer.public_key_b64 {
                                    Some(pk) => pk.clone(),
                                    None => {
                                        eprintln!(
                                            "❌ QUIC sender has no public key in Pack: {}",
                                            sender_fp
                                        );
                                        continue;
                                    }
                                },
                                None => {
                                    eprintln!("❌ QUIC sender not in Pack: {}", sender_fp);
                                    continue;
                                }
                            }
                        };

                        let nonce = match request_value["nonce"].as_str() {
                            Some(v) if !v.trim().is_empty() => v.trim().to_string(),
                            _ => {
                                eprintln!("missing QUIC nonce");
                                continue;
                            }
                        };

                        let signature = match request_value["signature"].as_str() {
                            Some(v) if !v.trim().is_empty() => v.trim().to_string(),
                            _ => {
                                eprintln!("❌ missing QUIC signature");
                                continue;
                            }
                        };

                        let signed_payload =
                            format!("fang.quic.open|{}|{}|{}", sender_fp, nonce, target);

                        if let Err(e) = verify_message(
                            &sender_public_key,
                            signed_payload.as_bytes(),
                            &signature,
                        ) {
                            eprintln!("❌ QUIC signature verification failed: {}", e);
                            continue;
                        }

                        {
                            let mut st = state.lock().await;

                            let replay_key = format!("quic|{}|{}", sender_fp, nonce);

                            if st.seen_nonces.contains_key(&replay_key) {
                                eprintln!("❌ QUIC replay detected: {}", replay_key);
                                continue;
                            }

                            st.seen_nonces.insert(replay_key, std::time::Instant::now());
                        }

                        println!("✅ QUIC sender trusted by Pack");
                        println!("🔐 QUIC signature verified");
                        println!("🧠 QUIC nonce accepted");
                        println!("🦷 Native QUIC target request: {}", target);

                        match tokio::time::timeout(
                            Duration::from_secs(5),
                            tokio::net::TcpStream::connect(&target),
                        )
                        .await
                        {
                            Ok(Ok(mut target_stream)) => {
                                println!("✅ target connected: {}", target);

                                let (mut target_read, mut target_write) = target_stream.split();

                                let up =
                                    async { tokio::io::copy(&mut recv, &mut target_write).await };

                                let down =
                                    async { tokio::io::copy(&mut target_read, &mut send).await };

                                let _ = tokio::join!(up, down);
                            }
                            Ok(Err(e)) => {
                                ww_error!(
                                    "QUIC",
                                    "TARGET_CONNECT_FAILED",
                                    "❌ target connect failed {}: {}",
                                    target,
                                    e
                                );
                            }
                            Err(_) => {
                                ww_error!(
                                    "QUIC",
                                    "TARGET_CONNECT_TIMEOUT",
                                    "❌ target connect timed out {} after 5s",
                                    target
                                );
                            }
                        }
                    }
                }
                Err(e) => eprintln!("QUIC connection failed: {}", e),
            }
        });
    }

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
