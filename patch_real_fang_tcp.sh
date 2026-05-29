#!/usr/bin/env bash
set -e

# Add clap to daemon
python3 - <<'PY'
from pathlib import Path

p = Path("crates/werewolfd/Cargo.toml")
s = p.read_text()

if 'clap = { version = "4", features = ["derive"] }' not in s:
    s = s.replace(
        'tracing-subscriber = "0.3"',
        'tracing-subscriber = "0.3"\nclap = { version = "4", features = ["derive"] }'
    )

p.write_text(s)
PY

cat > crates/werewolfd/src/main.rs <<'EOF'
use clap::Parser;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UnixListener, UnixStream},
    sync::Mutex,
};
use werewolf_core::{
    fang::{FangRecord, FangState},
    pack::{load_pack, save_pack, PeerRecord, TrustLevel},
    pelt::{generate_identity, load_identity, save_identity, PeltIdentity},
    protocol::{ControlRequest, ControlResponse},
    state::{Status, WolfMode},
};

#[derive(Parser, Debug, Clone)]
#[command(name = "werewolfd")]
struct Args {
    #[arg(long, default_value = "/tmp/werewolf.sock")]
    socket: String,

    #[arg(long, default_value = "~/.config/werewolf")]
    home: String,

    #[arg(long, default_value = "127.0.0.1:8443")]
    listen: String,
}

#[derive(Default)]
struct DaemonState {
    peers: Vec<PeerRecord>,
    fangs: Vec<FangRecord>,
    status: Status,
    pelt: Option<PeltIdentity>,
}

#[tokio::main]
async fn main() -> anyhow_free::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let home = expand_home(&args.home);

    let mut initial_state = DaemonState::default();

    let pelt_path = home.join("pelt.json");
    match load_identity(&pelt_path) {
        Ok(Some(identity)) => {
            println!("🐾 Pelt loaded: {}", identity.fingerprint);
            initial_state.status.pelt_ready = true;
            initial_state.pelt = Some(identity);
        }
        Ok(None) => {
            println!("🐾 No Pelt found. Run: werewolfctl pelt init");
        }
        Err(e) => {
            eprintln!("⚠️ Failed to load Pelt: {}", e);
        }
    }

    let pack_path = home.join("pack.json");
    match load_pack(&pack_path) {
        Ok(peers) => {
            println!("🐾 Loaded {} packmates", peers.len());
            initial_state.peers = peers;
        }
        Err(e) => {
            eprintln!("⚠️ Failed to load Pack: {}", e);
        }
    }

    let state = Arc::new(Mutex::new(initial_state));

    let net_state = state.clone();
    let listen_addr = args.listen.clone();
    tokio::spawn(async move {
        if let Err(e) = run_fang_listener(&listen_addr, net_state).await {
            eprintln!("🦷 Fang listener error: {}", e);
        }
    });

    let _ = fs::remove_file(&args.socket).await;
    let listener = UnixListener::bind(&args.socket)?;

    println!("🐺 werewolfd control socket: {}", args.socket);
    println!("🦷 Fang network listener: tcp://{}", args.listen);
    println!("🏠 Den home: {}", home.display());

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

async fn run_fang_listener(listen_addr: &str, _state: Arc<Mutex<DaemonState>>) -> io::Result<()> {
    let listener = TcpListener::bind(listen_addr).await?;

    loop {
        let (stream, peer_addr) = listener.accept().await?;

        tokio::spawn(async move {
            if let Err(e) = handle_fang_pipe(stream).await {
                eprintln!("fang pipe from {} error: {}", peer_addr, e);
            }
        });
    }
}

async fn handle_fang_pipe(stream: TcpStream) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    reader.read_line(&mut line).await?;
    let mut stream = reader.into_inner();

    let value: serde_json::Value = serde_json::from_str(&line)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let remote = value["remote"]
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing remote"))?;

    let mut remote_stream = TcpStream::connect(remote).await?;

    stream.write_all(b"{\"ok\":true}\n").await?;

    let _ = io::copy_bidirectional(&mut stream, &mut remote_stream).await?;

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

async fn handle_request(
    req: ControlRequest,
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
) -> ControlResponse {
    match req.cmd.as_str() {
        "status" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.status))
        }

        "pelt.init" => {
            let mut st = state.lock().await;
            let identity = generate_identity();
            let pelt_path = home.join("pelt.json");

            match save_identity(&pelt_path, &identity) {
                Ok(_) => {
                    st.status.pelt_ready = true;
                    st.pelt = Some(identity.clone());
                    ControlResponse::ok(req.id, json!({
                        "fingerprint": identity.fingerprint,
                        "saved_to": pelt_path
                    }))
                }
                Err(e) => ControlResponse::err(req.id, "PELT_SAVE_FAILED", e.to_string()),
            }
        }

        "pelt.fingerprint" => {
            let st = state.lock().await;
            match &st.pelt {
                Some(pelt) => ControlResponse::ok(req.id, json!({
                    "fingerprint": pelt.fingerprint
                })),
                None => ControlResponse::err(req.id, "NO_PELT", "No identity exists. Run pelt.init first."),
            }
        }

        "pack.add" => {
            let mut st = state.lock().await;

            let name = req.args["name"].as_str().unwrap_or("").trim().to_string();
            let fingerprint = req.args["fingerprint"].as_str().unwrap_or("").trim().to_string();
            let address = req.args["address"].as_str().unwrap_or("").trim().to_string();

            if name.is_empty() || fingerprint.is_empty() || address.is_empty() {
                return ControlResponse::err(req.id, "PACK_INVALID", "name, fingerprint and address are required");
            }

            if st.peers.iter().any(|p| p.name == name) {
                return ControlResponse::err(req.id, "PACK_DUP_NAME", format!("Peer name already exists: {}", name));
            }

            if st.peers.iter().any(|p| p.fingerprint == fingerprint) {
                return ControlResponse::err(req.id, "PACK_DUP_FINGERPRINT", format!("Fingerprint already exists: {}", fingerprint));
            }

            if !fingerprint.starts_with("wwp1:") {
                return ControlResponse::err(req.id, "PACK_BAD_FINGERPRINT", "fingerprint must start with wwp1:");
            }

            let peer = PeerRecord {
                name,
                fingerprint,
                address,
                trust: TrustLevel::Packmate,
            };

            st.peers.push(peer);

            let path = home.join("pack.json");

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

        "pack.remove" => {
            let mut st = state.lock().await;
            let name = req.args["name"].as_str().unwrap_or("").trim().to_string();

            if name.is_empty() {
                return ControlResponse::err(req.id, "PACK_INVALID", "name is required");
            }

            let before = st.peers.len();
            st.peers.retain(|p| p.name != name);

            if st.peers.len() == before {
                return ControlResponse::err(req.id, "PACK_NOT_FOUND", format!("Peer not found: {}", name));
            }

            let path = home.join("pack.json");

            match save_pack(&path, &st.peers) {
                Ok(_) => ControlResponse::ok(req.id, json!({
                    "status": "removed",
                    "packmates": st.peers.len()
                })),
                Err(e) => ControlResponse::err(req.id, "PACK_SAVE_FAILED", e.to_string()),
            }
        }

        "fang.open" => {
            let mut st = state.lock().await;

            let peer = req.args["peer"].as_str().unwrap_or("").trim().to_string();
            let local = req.args["local"].as_str().unwrap_or("").trim().to_string();
            let remote = req.args["remote"].as_str().unwrap_or("").trim().to_string();

            if peer.is_empty() || local.is_empty() || remote.is_empty() {
                return ControlResponse::err(req.id, "FANG_INVALID", "peer, local and remote are required");
            }

            let peer_record = match st.peers.iter().find(|p| p.name == peer) {
                Some(p) => p.clone(),
                None => return ControlResponse::err(req.id, "FANG_UNKNOWN_PEER", format!("Unknown peer: {}", peer)),
            };

            let peer_addr = match parse_tcp_address(&peer_record.address) {
                Ok(a) => a,
                Err(e) => return ControlResponse::err(req.id, "FANG_BAD_PEER_ADDRESS", e),
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

            tokio::spawn(async move {
                if let Err(e) = run_local_fang_forwarder(&fang_id, &local, &peer_addr, &remote).await {
                    eprintln!("🦷 Fang {} failed: {}", fang_id, e);
                }
            });

            ControlResponse::ok(req.id, json!({
                "fang_id": format!("fang_{}", st.fangs.len()),
                "state": "active",
                "local": local,
                "peer": peer,
                "remote": remote
            }))
        }

        "fang.list" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.fangs))
        }

        "fang.close" => {
            let mut st = state.lock().await;
            let fang_id = req.args["fang_id"].as_str().unwrap_or("").trim().to_string();

            if fang_id.is_empty() {
                return ControlResponse::err(req.id, "FANG_INVALID", "fang_id is required");
            }

            let before = st.fangs.len();
            st.fangs.retain(|f| f.id != fang_id);

            if st.fangs.len() == before {
                return ControlResponse::err(req.id, "FANG_NOT_FOUND", format!("Fang not found: {}", fang_id));
            }

            st.status.active_fangs = st.fangs.len();

            if st.fangs.is_empty() {
                st.status.mode = WolfMode::Human;
            }

            ControlResponse::ok(req.id, json!({
                "status": "closed",
                "active_fangs": st.fangs.len()
            }))
        }

        _ => ControlResponse::err(req.id, "UNKNOWN_CMD", format!("Unknown command: {}", req.cmd)),
    }
}

async fn run_local_fang_forwarder(
    fang_id: &str,
    local: &str,
    peer_addr: &str,
    remote: &str,
) -> io::Result<()> {
    let listener = TcpListener::bind(local).await?;
    println!("🦷 {} listening locally on {}", fang_id, local);

    loop {
        let (mut inbound, client_addr) = listener.accept().await?;
        let peer_addr = peer_addr.to_string();
        let remote = remote.to_string();
        let fang_id = fang_id.to_string();

        tokio::spawn(async move {
            if let Err(e) = pipe_one_fang_connection(&mut inbound, &peer_addr, &remote).await {
                eprintln!("🦷 {} client {} pipe error: {}", fang_id, client_addr, e);
            }
        });
    }
}

async fn pipe_one_fang_connection(
    inbound: &mut TcpStream,
    peer_addr: &str,
    remote: &str,
) -> io::Result<()> {
    let mut outbound = TcpStream::connect(peer_addr).await?;

    let hello = json!({
        "cmd": "fang.pipe",
        "remote": remote
    });

    outbound.write_all(serde_json::to_string(&hello).unwrap().as_bytes()).await?;
    outbound.write_all(b"\n").await?;

    let mut reader = BufReader::new(outbound);
    let mut ack = String::new();
    reader.read_line(&mut ack).await?;
    let mut outbound = reader.into_inner();

    let _ = io::copy_bidirectional(inbound, &mut outbound).await?;

    Ok(())
}

fn parse_tcp_address(address: &str) -> Result<String, String> {
    if let Some(rest) = address.strip_prefix("tcp://") {
        return Ok(rest.to_string());
    }

    if address.starts_with("quic://") {
        return Err("quic:// is reserved for the next encrypted transport patch. Use tcp:// for this test.".to_string());
    }

    Err("peer address must start with tcp:// for this transport test".to_string())
}

fn expand_home(input: &str) -> PathBuf {
    if input == "~" {
        return PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));
    }

    if let Some(rest) = input.strip_prefix("~/") {
        return PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string())).join(rest);
    }

    PathBuf::from(input)
}

mod anyhow_free {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
EOF
