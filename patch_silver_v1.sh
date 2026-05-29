#!/usr/bin/env bash
set -e

python3 - <<'PY'
from pathlib import Path

# Patch CLI
p = Path("crates/werewolfctl/src/main.rs")
s = p.read_text()

s = s.replace(
'''enum Commands {
    Status,''',
'''enum Commands {
    Status,

    Silver {
        #[command(subcommand)]
        command: SilverCommands,
    },'''
)

s = s.replace(
'''#[derive(Subcommand)]
enum PeltCommands {''',
'''#[derive(Subcommand)]
enum SilverCommands {
    Trigger,
    Reset,
}

#[derive(Subcommand)]
enum PeltCommands {'''
)

s = s.replace(
'''Commands::Status => ("status".to_string(), json!({})),''',
'''Commands::Status => ("status".to_string(), json!({})),
        Commands::Silver { command } => match command {
            SilverCommands::Trigger => ("silver.trigger".to_string(), json!({})),
            SilverCommands::Reset => ("silver.reset".to_string(), json!({})),
        },'''
)

p.write_text(s)

# Patch daemon
p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

s = s.replace(
'''        "fang.open" => {
            let mut st = state.lock().await;''',
'''        "fang.open" => {
            let mut st = state.lock().await;

            if matches!(st.status.mode, WolfMode::Silver) {
                return ControlResponse::err(
                    req.id,
                    "SILVER_ACTIVE",
                    "Silver mode is active. Fang open rejected.",
                );
            }'''
)

s = s.replace(
'''        "fang.list" => {''',
'''        "silver.trigger" => {
            let mut st = state.lock().await;
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

        "fang.list" => {'''
)

p.write_text(s)
PY

echo "🥈 Silver v1 patch applied."
