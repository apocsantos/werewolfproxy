#!/usr/bin/env bash
set -e

mkdir -p crates/werewolf-core/src
mkdir -p crates/werewolfd/src
mkdir -p crates/werewolfctl/src
mkdir -p docs/lore

cat > Cargo.toml <<'EOF'
[workspace]
members = [
  "crates/werewolf-core",
  "crates/werewolfd",
  "crates/werewolfctl"
]
resolver = "2"
EOF

cat > crates/werewolf-core/Cargo.toml <<'EOF'
[package]
name = "werewolf-core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
blake3 = "1"
ed25519-dalek = { version = "2", features = ["rand_core"] }
rand_core = { version = "0.6", features = ["getrandom"] }
base64 = "0.22"
toml = "0.8"
EOF

cat > crates/werewolf-core/src/lib.rs <<'EOF'
pub mod protocol;
pub mod state;
pub mod pelt;
EOF

cat > crates/werewolf-core/src/protocol.rs <<'EOF'
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct ControlRequest {
    pub id: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ControlResponse {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ControlError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ControlError {
    pub code: String,
    pub message: String,
}

impl ControlResponse {
    pub fn ok(id: impl Into<String>, result: Value) -> Self {
        Self { id: id.into(), ok: true, result: Some(result), error: None }
    }

    pub fn err(id: impl Into<String>, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ok: false,
            result: None,
            error: Some(ControlError { code: code.into(), message: message.into() }),
        }
    }
}
EOF

cat > crates/werewolf-core/src/state.rs <<'EOF'
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WolfMode {
    Human,
    Wolf,
    Silver,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub mode: WolfMode,
    pub pelt_ready: bool,
    pub packmates: usize,
    pub active_fangs: usize,
    pub silver: String,
    pub hide: String,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            mode: WolfMode::Human,
            pelt_ready: false,
            packmates: 0,
            active_fangs: 0,
            silver: "armed".to_string(),
            hide: "light".to_string(),
        }
    }
}
EOF

cat > crates/werewolf-core/src/pelt.rs <<'EOF'
use base64::{engine::general_purpose::STANDARD, Engine};
use blake3::Hasher;
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeltIdentity {
    pub public_key_b64: String,
    pub secret_key_b64: String,
    pub fingerprint: String,
}

pub fn generate_identity() -> PeltIdentity {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let public = verifying_key.to_bytes();
    let secret = signing_key.to_bytes();

    let mut hasher = Hasher::new();
    hasher.update(&public);
    let hash = hasher.finalize();

    let fp_hex = hexish(&hash.as_bytes()[0..8]);
    let fingerprint = format!("wwp1:{}", fp_hex);

    PeltIdentity {
        public_key_b64: STANDARD.encode(public),
        secret_key_b64: STANDARD.encode(secret),
        fingerprint,
    }
}

fn hexish(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join("-")
}
EOF

cat > crates/werewolfd/Cargo.toml <<'EOF'
[package]
name = "werewolfd"
version = "0.1.0"
edition = "2021"

[dependencies]
werewolf-core = { path = "../werewolf-core" }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
EOF

cat > crates/werewolfd/src/main.rs <<'EOF'
use serde_json::json;
use std::sync::Arc;
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Mutex,
};
use werewolf_core::{
    pelt::{generate_identity, PeltIdentity},
    protocol::{ControlRequest, ControlResponse},
    state::Status,
};

const SOCKET_PATH: &str = "/tmp/werewolf.sock";

#[derive(Default)]
struct DaemonState {
    status: Status,
    pelt: Option<PeltIdentity>,
}

#[tokio::main]
async fn main() -> anyhow_free::Result<()> {
    tracing_subscriber::fmt::init();

    let _ = fs::remove_file(SOCKET_PATH).await;
    let listener = UnixListener::bind(SOCKET_PATH)?;

    println!("🐺 werewolfd listening on {}", SOCKET_PATH);

    let state = Arc::new(Mutex::new(DaemonState::default()));

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, state).await {
                eprintln!("client error: {}", e);
            }
        });
    }
}

async fn handle_client(stream: UnixStream, state: Arc<Mutex<DaemonState>>) -> std::io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<ControlRequest>(&line) {
            Ok(req) => handle_request(req, state.clone()).await,
            Err(e) => ControlResponse::err("unknown", "BAD_JSON", e.to_string()),
        };

        let encoded = serde_json::to_string(&response).unwrap();
        writer.write_all(encoded.as_bytes()).await?;
        writer.write_all(b"\n").await?;
    }

    Ok(())
}

async fn handle_request(req: ControlRequest, state: Arc<Mutex<DaemonState>>) -> ControlResponse {
    match req.cmd.as_str() {
        "status" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.status))
        }

        "pelt.init" => {
            let mut st = state.lock().await;
            let identity = generate_identity();
            st.status.pelt_ready = true;
            st.pelt = Some(identity.clone());
            ControlResponse::ok(req.id, json!({
                "fingerprint": identity.fingerprint
            }))
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

        _ => ControlResponse::err(req.id, "UNKNOWN_CMD", format!("Unknown command: {}", req.cmd)),
    }
}

// tiny local alias so we avoid adding anyhow yet
mod anyhow_free {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
EOF

cat > crates/werewolfctl/Cargo.toml <<'EOF'
[package]
name = "werewolfctl"
version = "0.1.0"
edition = "2021"

[dependencies]
werewolf-core = { path = "../werewolf-core" }
tokio = { version = "1", features = ["full"] }
clap = { version = "4", features = ["derive"] }
serde_json = "1"
EOF

cat > crates/werewolfctl/src/main.rs <<'EOF'
use clap::{Parser, Subcommand};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};
use werewolf_core::protocol::{ControlRequest, ControlResponse};

const SOCKET_PATH: &str = "/tmp/werewolf.sock";

#[derive(Parser)]
#[command(name = "werewolfctl")]
#[command(about = "WerewolfProxy control tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Status,
    Pelt {
        #[command(subcommand)]
        command: PeltCommands,
    },
}

#[derive(Subcommand)]
enum PeltCommands {
    Init,
    Fingerprint,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = Cli::parse();

    let cmd = match cli.command {
        Commands::Status => "status".to_string(),
        Commands::Pelt { command } => match command {
            PeltCommands::Init => "pelt.init".to_string(),
            PeltCommands::Fingerprint => "pelt.fingerprint".to_string(),
        },
    };

    let response = send_request(&cmd).await?;
    print_response(response);

    Ok(())
}

async fn send_request(cmd: &str) -> std::io::Result<ControlResponse> {
    let mut stream = UnixStream::connect(SOCKET_PATH).await?;

    let req = ControlRequest {
        id: "req-001".to_string(),
        cmd: cmd.to_string(),
        args: json!({}),
    };

    let encoded = serde_json::to_string(&req).unwrap();
    stream.write_all(encoded.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let response: ControlResponse = serde_json::from_str(&line).unwrap();
    Ok(response)
}

fn print_response(resp: ControlResponse) {
    if resp.ok {
        println!("{}", serde_json::to_string_pretty(&resp.result).unwrap());
    } else {
        println!("{}", serde_json::to_string_pretty(&resp.error).unwrap());
    }
}
EOF

cat > docs/lore/architecture.md <<'EOF'
# WerewolfProxy Architecture

WerewolfProxy is a Linux-first private reverse-tunnel overlay.

## Mythic Modules

- Moon: discovery/rendezvous
- Pelt: identity and keys
- Pack: trusted peers
- Fang: encrypted tunnel engine
- Hide: metadata minimization
- Silver: lockdown mode
- Den: local configuration
- Alpha: policy engine
- Howl: sanitized logs/events
EOF

echo "🐺 WerewolfProxy skeleton created."
echo "Run:"
echo "cargo build"
echo "cargo run -p werewolfd"
echo "In another terminal:"
echo "cargo run -p werewolfctl -- status"
echo "cargo run -p werewolfctl -- pelt init"
echo "cargo run -p werewolfctl -- pelt fingerprint"
