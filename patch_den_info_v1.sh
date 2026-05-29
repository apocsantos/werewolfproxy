#!/usr/bin/env bash
set -e

python3 - <<'PY'
from pathlib import Path

p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

# Add den fields to DaemonState
s = s.replace(
'''struct DaemonState {
    peers: Vec<PeerRecord>,
    fangs: Vec<FangRecord>,
    fang_profiles: Vec<FangProfile>,
    fang_tasks: HashMap<String, JoinHandle<()>>,
    seen_nonces: HashMap<String, Instant>,
    status: Status,
    pelt: Option<PeltIdentity>,
}''',
'''struct DaemonState {
    peers: Vec<PeerRecord>,
    fangs: Vec<FangRecord>,
    fang_profiles: Vec<FangProfile>,
    fang_tasks: HashMap<String, JoinHandle<()>>,
    seen_nonces: HashMap<String, Instant>,
    status: Status,
    pelt: Option<PeltIdentity>,
    den_socket: String,
    den_home: String,
    den_listen: String,
}'''
)

# Set den fields after initial_state creation
s = s.replace(
'''    let mut initial_state = DaemonState::default();''',
'''    let mut initial_state = DaemonState::default();
    initial_state.den_socket = args.socket.clone();
    initial_state.den_home = home.display().to_string();
    initial_state.den_listen = args.listen.clone();'''
)

# Add den.info command before status
s = s.replace(
'''        "status" => {
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.status))
        }''',
'''        "den.info" => {
            let st = state.lock().await;
            ControlResponse::ok(
                req.id,
                json!({
                    "socket": st.den_socket,
                    "home": st.den_home,
                    "listen": st.den_listen,
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
            let st = state.lock().await;
            ControlResponse::ok(req.id, json!(st.status))
        }'''
)

p.write_text(s)

# Patch CLI
p = Path("crates/werewolfctl/src/main.rs")
s = p.read_text()

s = s.replace(
'''enum Commands {
    Status,''',
'''enum Commands {
    Status,

    Den {
        #[command(subcommand)]
        command: DenCommands,
    },'''
)

s = s.replace(
'''#[derive(Subcommand)]
enum SilverCommands {''',
'''#[derive(Subcommand)]
enum DenCommands {
    Info,
}

#[derive(Subcommand)]
enum SilverCommands {'''
)

s = s.replace(
'''Commands::Status => ("status".to_string(), json!({})),''',
'''Commands::Status => ("status".to_string(), json!({})),
        Commands::Den { command } => match command {
            DenCommands::Info => ("den.info".to_string(), json!({})),
        },'''
)

p.write_text(s)
PY

echo "🏠 Den Info v1 patch applied."
