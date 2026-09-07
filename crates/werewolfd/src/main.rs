mod state;
use state::DaemonState;
mod cli;
use cli::Args;
mod quic_fang;
use crate::quic_fang::open_quic_fang;
use crate::quic_lab::make_server_endpoint;
mod quic_lab;
use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use clap::Parser;
use rand_core::{OsRng, RngCore};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UnixListener, UnixStream},
    sync::Mutex,
};
use werewolf_core::{
    fang::{FangRecord, FangState},
    fang_profile::{save_fang_profiles, FangProfile},
    pack::{save_pack, PeerRecord, TrustLevel},
    pelt::{
        fingerprint_from_public_key_b64, generate_identity, save_identity, sign_message,
        verify_message, PeltIdentity,
    },
    protocol::{ControlRequest, ControlResponse},
    state::WolfMode,
};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

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
use persistence::{
    forget_active_fang_profile, load_active_fang_profiles, load_startup_state,
    remember_active_fang_profile,
};

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
        if let Err(e) = run_fang_listener(&listen_addr, net_state).await {
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

    let _ = fs::remove_file(&args.socket).await;
    let listener = UnixListener::bind(&args.socket)?;

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

async fn run_fang_listener(listen_addr: &str, state: Arc<Mutex<DaemonState>>) -> io::Result<()> {
    let listener = TcpListener::bind(listen_addr).await?;

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let state_for_client = state.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_fang_pipe(stream, state_for_client).await {
                eprintln!("fang pipe from {} error: {}", peer_addr, e);
            }
        });
    }
}

async fn handle_fang_pipe(stream: TcpStream, state: Arc<Mutex<DaemonState>>) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    reader.read_line(&mut line).await?;
    let mut stream = reader.into_inner();

    let value: serde_json::Value = serde_json::from_str(&line)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let remote = value["remote"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing remote"))?;

    let sender_pubkey = value["sender_pubkey"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing sender_pubkey"))?;

    let sender_fingerprint = value["sender_fingerprint"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing sender_fingerprint"))?;

    let client_x25519 = value["client_x25519"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing client_x25519"))?;

    let nonce = value["nonce"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing nonce"))?;

    let signature = value["signature"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing signature"))?;

    let derived_fp = fingerprint_from_public_key_b64(sender_pubkey)
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    if derived_fp != sender_fingerprint {
        stream
            .write_all(b"{\"ok\":false,\"error\":\"fingerprint mismatch\"}\n")
            .await?;
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "fingerprint mismatch",
        ));
    }

    let signed_text = format!(
        "fang.pipe|{}|{}|{}|{}",
        sender_fingerprint, remote, nonce, client_x25519
    );

    if let Err(e) = verify_message(sender_pubkey, signed_text.as_bytes(), signature) {
        stream
            .write_all(b"{\"ok\":false,\"error\":\"bad signature\"}\n")
            .await?;
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, e));
    }

    {
        let mut st = state.lock().await;

        if !st.peers.iter().any(|p| p.fingerprint == sender_fingerprint) {
            stream
                .write_all(b"{\"ok\":false,\"error\":\"sender not in pack\"}\n")
                .await?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "sender not in pack",
            ));
        }

        const REPLAY_WINDOW: Duration = Duration::from_secs(300);

        let now = Instant::now();

        st.seen_nonces
            .retain(|_, seen_at| now.duration_since(*seen_at) < REPLAY_WINDOW);

        let replay_key = format!("{}|{}", sender_fingerprint, nonce);

        if st.seen_nonces.contains_key(&replay_key) {
            stream
                .write_all(b"{\"ok\":false,\"error\":\"replay detected\"}\n")
                .await?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "replay detected",
            ));
        }

        st.seen_nonces.insert(replay_key, now);
    }

    let receiver_identity = {
        let st = state.lock().await;
        st.pelt.clone().ok_or_else(|| {
            io::Error::new(io::ErrorKind::PermissionDenied, "receiver has no Pelt")
        })?
    };

    let server_secret = StaticSecret::random_from_rng(OsRng);
    let server_public = X25519PublicKey::from(&server_secret);
    let server_public_b64 = STANDARD.encode(server_public.as_bytes());

    let ack_text = format!(
        "fang.ack|{}|{}|{}|{}",
        receiver_identity.fingerprint, sender_fingerprint, nonce, server_public_b64
    );

    let ack_signature = sign_message(&receiver_identity, ack_text.as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let remote_stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(remote))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "remote connect timed out"))??;

    let ack = json!({
        "ok": true,
        "session": "fang-v1-secure",
        "receiver_pubkey": receiver_identity.public_key_b64,
        "receiver_fingerprint": receiver_identity.fingerprint,
        "server_x25519": server_public_b64,
        "nonce": nonce,
        "signature": ack_signature
    });

    stream
        .write_all(serde_json::to_string(&ack).unwrap().as_bytes())
        .await?;
    stream.write_all(b"\n").await?;

    let key = derive_shared_key(&server_secret, client_x25519)?;

    secure_copy_server_side(stream, remote_stream, key).await?;

    Ok(())
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
            run_plain_tcp_forwarder(&task_fang_id, &task_local, &task_remote).await
        } else {
            run_local_fang_forwarder(
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

async fn run_plain_tcp_forwarder(fang_id: &str, local: &str, remote: &str) -> io::Result<()> {
    let listener = TcpListener::bind(local).await?;
    println!("🦷 {} plain TCP listening locally on {}", fang_id, local);

    loop {
        let (mut inbound, client_addr) = listener.accept().await?;
        let remote = remote.to_string();
        let fang_id = fang_id.to_string();

        tokio::spawn(async move {
            match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&remote)).await {
                Ok(Ok(mut outbound)) => {
                    if let Err(e) = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await
                    {
                        eprintln!(
                            "🦷 {} plain client {} pipe error: {}",
                            fang_id, client_addr, e
                        );
                    }
                }
                Ok(Err(e)) => {
                    eprintln!(
                        "🦷 {} plain target connect error {}: {}",
                        fang_id, remote, e
                    );
                }
                Err(_) => {
                    eprintln!(
                        "🦷 {} plain target connect timed out after 5s: {}",
                        fang_id, remote
                    );
                }
            }
        });
    }
}

async fn run_local_fang_forwarder(
    fang_id: &str,
    local: &str,
    peer_addr: &str,
    remote: &str,
    identity: PeltIdentity,
) -> io::Result<()> {
    let listener = TcpListener::bind(local).await?;
    println!("🦷 {} listening locally on {}", fang_id, local);

    loop {
        let (mut inbound, client_addr) = listener.accept().await?;
        let peer_addr = peer_addr.to_string();
        let remote = remote.to_string();
        let fang_id = fang_id.to_string();
        let identity = identity.clone();

        tokio::spawn(async move {
            if let Err(e) =
                pipe_one_fang_connection(&mut inbound, &peer_addr, &remote, identity).await
            {
                eprintln!("🦷 {} client {} pipe error: {}", fang_id, client_addr, e);
            }
        });
    }
}

async fn pipe_one_fang_connection(
    inbound: &mut TcpStream,
    peer_addr: &str,
    remote: &str,
    identity: PeltIdentity,
) -> io::Result<()> {
    let mut outbound = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(peer_addr))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "peer connect timed out"))??;

    let nonce = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    let client_secret = StaticSecret::random_from_rng(OsRng);
    let client_public = X25519PublicKey::from(&client_secret);
    let client_public_b64 = STANDARD.encode(client_public.as_bytes());

    let signed_text = format!(
        "fang.pipe|{}|{}|{}|{}",
        identity.fingerprint, remote, nonce, client_public_b64
    );

    let signature = sign_message(&identity, signed_text.as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let hello = json!({
        "cmd": "fang.pipe",
        "remote": remote,
        "sender_pubkey": identity.public_key_b64,
        "sender_fingerprint": identity.fingerprint,
        "client_x25519": client_public_b64,
        "nonce": nonce,
        "signature": signature
    });

    outbound
        .write_all(serde_json::to_string(&hello).unwrap().as_bytes())
        .await?;
    outbound.write_all(b"\n").await?;

    let mut reader = BufReader::new(outbound);
    let mut ack = String::new();
    reader.read_line(&mut ack).await?;
    let outbound = reader.into_inner();

    let ack_value: serde_json::Value = serde_json::from_str(&ack)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    if ack_value["ok"].as_bool() != Some(true) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("fang rejected: {}", ack),
        ));
    }

    let receiver_pubkey = ack_value["receiver_pubkey"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing receiver_pubkey"))?;

    let receiver_fingerprint = ack_value["receiver_fingerprint"].as_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing receiver_fingerprint")
    })?;

    let server_x25519 = ack_value["server_x25519"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing server_x25519"))?;

    let ack_nonce = ack_value["nonce"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing ack nonce"))?;

    let ack_signature = ack_value["signature"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing ack signature"))?;

    if ack_nonce != nonce {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "ack nonce mismatch",
        ));
    }

    let derived_receiver_fp = fingerprint_from_public_key_b64(receiver_pubkey)
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    if derived_receiver_fp != receiver_fingerprint {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "receiver fingerprint mismatch",
        ));
    }

    let expected_ack_text = format!(
        "fang.ack|{}|{}|{}|{}",
        receiver_fingerprint, identity.fingerprint, nonce, server_x25519
    );

    verify_message(receiver_pubkey, expected_ack_text.as_bytes(), ack_signature)
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    let key = derive_shared_key(&client_secret, server_x25519)?;

    secure_copy_client_side(inbound, outbound, key).await?;

    Ok(())
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

fn derive_shared_key(secret: &StaticSecret, peer_public_b64: &str) -> io::Result<[u8; 32]> {
    let peer_bytes = STANDARD
        .decode(peer_public_b64)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let peer_arr: [u8; 32] = peer_bytes
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad x25519 public key length"))?;

    let peer_public = X25519PublicKey::from(peer_arr);
    let shared = secret.diffie_hellman(&peer_public);

    let hash = blake3::hash(shared.as_bytes());
    Ok(*hash.as_bytes())
}

fn make_nonce(direction: u8, counter: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[0] = direction;
    nonce[4..12].copy_from_slice(&counter.to_be_bytes());
    nonce
}

async fn write_encrypted_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    key: &[u8; 32],
    direction: u8,
    counter: &mut u64,
    plaintext: &[u8],
) -> io::Result<()> {
    const MIN_FRAME_SIZE: usize = 768;
    const MAX_FRAME_SIZE: usize = 2048;
    const STEP: usize = 128;

    if plaintext.len() > MAX_FRAME_SIZE - 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plaintext frame exceeds hide limit",
        ));
    }

    let minimum_needed = plaintext.len() + 2;
    let min_bucket = minimum_needed.max(MIN_FRAME_SIZE);
    let buckets = ((MAX_FRAME_SIZE - min_bucket) / STEP) + 1;

    let random_bucket = if buckets > 1 {
        (OsRng.next_u32() as usize) % buckets
    } else {
        0
    };

    let frame_size = min_bucket + (random_bucket * STEP);

    let mut padded_plaintext = Vec::with_capacity(frame_size);
    padded_plaintext.extend_from_slice(&(plaintext.len() as u16).to_be_bytes());
    padded_plaintext.extend_from_slice(plaintext);
    padded_plaintext.resize(frame_size, 0);

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce_bytes = make_nonce(direction, *counter);
    *counter += 1;

    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), padded_plaintext.as_ref())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "encrypt failed"))?;

    let len = ciphertext.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&ciphertext).await?;
    writer.flush().await?;

    Ok(())
}

async fn read_encrypted_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    key: &[u8; 32],
    direction: u8,
    counter: &mut u64,
) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];

    if reader.read_exact(&mut len_buf).await.is_err() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed"));
    }

    let len = u32::from_be_bytes(len_buf) as usize;

    if len > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }

    let mut ciphertext = vec![0u8; len];
    reader.read_exact(&mut ciphertext).await?;

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce_bytes = make_nonce(direction, *counter);
    *counter += 1;

    let padded_plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "decrypt failed"))?;

    if padded_plaintext.len() < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad frame"));
    }

    let real_len = u16::from_be_bytes([padded_plaintext[0], padded_plaintext[1]]) as usize;

    if real_len > padded_plaintext.len() - 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad plaintext length",
        ));
    }

    Ok(padded_plaintext[2..2 + real_len].to_vec())
}

async fn secure_copy_client_side(
    inbound: &mut TcpStream,
    outbound: TcpStream,
    key: [u8; 32],
) -> io::Result<()> {
    let (mut in_r, mut in_w) = inbound.split();
    let (mut out_r, mut out_w) = outbound.into_split();

    let client_to_server = async {
        let mut buf = vec![0u8; 1400];
        let mut counter = 0u64;

        loop {
            let n = in_r.read(&mut buf).await?;
            if n == 0 {
                let _ = out_w.shutdown().await;
                return Ok::<(), io::Error>(());
            }

            eprintln!("TCPV2 client->server plaintext={} counter={}", n, counter);
            write_encrypted_frame(&mut out_w, &key, 0, &mut counter, &buf[..n]).await?;
            eprintln!("TCPV2 client->server sent counter={}", counter);
        }
    };

    let server_to_client = async {
        let mut counter = 0u64;

        loop {
            eprintln!("TCPV2 client waiting server->client counter={}", counter);
            match tokio::time::timeout(
                Duration::from_secs(60),
                read_encrypted_frame(&mut out_r, &key, 1, &mut counter),
            )
            .await
            {
                Ok(Ok(plaintext)) => {
                    eprintln!(
                        "TCPV2 client recv server->client plaintext={} counter={}",
                        plaintext.len(),
                        counter
                    );
                    in_w.write_all(&plaintext).await?;
                    in_w.flush().await?;
                }
                Ok(Err(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    let _ = in_w.shutdown().await;
                    return Ok::<(), io::Error>(());
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "tcp-v2 client read idle timeout",
                    ));
                }
            }
        }
    };

    let _ = tokio::join!(client_to_server, server_to_client);
    Ok(())
}

async fn secure_copy_server_side(
    stream: TcpStream,
    remote_stream: TcpStream,
    key: [u8; 32],
) -> io::Result<()> {
    let (mut fang_r, mut fang_w) = stream.into_split();
    let (mut remote_r, mut remote_w) = remote_stream.into_split();

    let client_to_remote = async {
        let mut counter = 0u64;

        loop {
            eprintln!("TCPV2 server waiting client->remote counter={}", counter);
            match tokio::time::timeout(
                Duration::from_secs(60),
                read_encrypted_frame(&mut fang_r, &key, 0, &mut counter),
            )
            .await
            {
                Ok(Ok(plaintext)) => {
                    eprintln!(
                        "TCPV2 server recv client->remote plaintext={} counter={}",
                        plaintext.len(),
                        counter
                    );
                    remote_w.write_all(&plaintext).await?;
                    remote_w.flush().await?;
                }
                Ok(Err(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    let _ = remote_w.shutdown().await;
                    return Ok::<(), io::Error>(());
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "tcp-v2 server read idle timeout",
                    ));
                }
            }
        }
    };

    let remote_to_client = async {
        let mut buf = vec![0u8; 1400];
        let mut counter = 0u64;

        loop {
            let n = remote_r.read(&mut buf).await?;
            if n == 0 {
                let _ = fang_w.shutdown().await;
                return Ok::<(), io::Error>(());
            }

            eprintln!("TCPV2 server->client plaintext={} counter={}", n, counter);
            write_encrypted_frame(&mut fang_w, &key, 1, &mut counter, &buf[..n]).await?;
            eprintln!("TCPV2 server->client sent counter={}", counter);
        }
    };

    let _ = tokio::join!(client_to_remote, remote_to_client);
    Ok(())
}

mod anyhow_free {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
