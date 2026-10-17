#!/usr/bin/env bash
set -e

python3 - <<'PY'
from pathlib import Path

p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

# Add HashMap import alongside HashSet
s = s.replace(
    "use std::{collections::HashSet, path::PathBuf, sync::Arc};",
    "use std::{collections::{HashMap, HashSet}, path::PathBuf, sync::Arc};"
)

# Import JoinHandle
s = s.replace(
    "sync::Mutex,",
    "sync::Mutex,\n    task::JoinHandle,"
)

# Add fang_tasks to state
s = s.replace(
'''struct DaemonState {
    peers: Vec<PeerRecord>,
    fangs: Vec<FangRecord>,
    seen_nonces: HashSet<String>,
    status: Status,
    pelt: Option<PeltIdentity>,
}''',
'''struct DaemonState {
    peers: Vec<PeerRecord>,
    fangs: Vec<FangRecord>,
    fang_tasks: HashMap<String, JoinHandle<()>>,
    seen_nonces: HashSet<String>,
    status: Status,
    pelt: Option<PeltIdentity>,
}'''
)

# Replace spawn block in fang.open
old = '''            tokio::spawn(async move {
                if let Err(e) = run_local_fang_forwarder(
                    &task_fang_id,
                    &task_local,
                    &task_peer_addr,
                    &task_remote,
                    task_identity,
                )
                .await
                {
                    eprintln!("🦷 Fang {} failed: {}", task_fang_id, e);
                }
            });

            ControlResponse::ok('''

new = '''            let handle = tokio::spawn(async move {
                if let Err(e) = run_local_fang_forwarder(
                    &task_fang_id,
                    &task_local,
                    &task_peer_addr,
                    &task_remote,
                    task_identity,
                )
                .await
                {
                    eprintln!("🦷 Fang {} failed: {}", task_fang_id, e);
                }
            });

            st.fang_tasks.insert(fang_id.clone(), handle);

            ControlResponse::ok('''

if old not in s:
    raise SystemExit("Could not find fang.open spawn block")

s = s.replace(old, new)

# Patch fang.close to abort task
old = '''            let before = st.fangs.len();
            st.fangs.retain(|f| f.id != fang_id);

            if st.fangs.len() == before {
                return ControlResponse::err(
                    req.id,
                    "FANG_NOT_FOUND",
                    format!("Fang not found: {}", fang_id),
                );
            }

            st.status.active_fangs = st.fangs.len();'''

new = '''            let before = st.fangs.len();
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

            st.status.active_fangs = st.fangs.len();'''

if old not in s:
    raise SystemExit("Could not find fang.close block")

s = s.replace(old, new)

# Patch silver.trigger to abort all tasks
old = '''            let mut st = state.lock().await;
            st.fangs.clear();
            st.status.active_fangs = 0;
            st.status.mode = WolfMode::Silver;
            st.status.silver = "active".to_string();'''

new = '''            let mut st = state.lock().await;

            for (_, handle) in st.fang_tasks.drain() {
                handle.abort();
            }

            st.fangs.clear();
            st.status.active_fangs = 0;
            st.status.mode = WolfMode::Silver;
            st.status.silver = "active".to_string();'''

if old not in s:
    raise SystemExit("Could not find silver.trigger block")

s = s.replace(old, new)

p.write_text(s)
PY

echo "🦷 Fang cancellation v1 patch applied."
