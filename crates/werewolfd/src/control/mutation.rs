//! Control mutations enter through the single admission coordinator. Candidate
//! state is prepared off the live model, persisted off the Tokio worker, then
//! published without a fallible step. Client lifetime does not own this task.
use crate::state::DaemonState;
use serde::Serialize;
use serde_json::json;
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;
use werewolf_core::{
    fang_profile::FangProfile,
    local_fs::{CommitOutcome, PrivateDirectory},
    pack::{PeerRecord, TrustLevel},
    pelt::generate_identity,
    protocol::{ControlRequest, ControlResponse},
    state_validation,
};

pub(super) fn handles(command: &str) -> bool {
    matches!(
        command,
        "pelt.init"
            | "pack.add"
            | "pack.set_address"
            | "pack.revoke"
            | "pack.remove"
            | "fang.profile.add"
            | "fang.profile.remove"
    )
}
fn error(id: &str, code: &str) -> ControlResponse {
    ControlResponse::err(id, code, "local state mutation rejected")
}
fn arg(req: &ControlRequest, field: &str) -> String {
    req.args[field].as_str().unwrap_or("").trim().to_owned()
}

async fn durable<T: Serialize + Sync>(
    home: &Path,
    file: &'static str,
    candidate: &T,
    create_only: bool,
    state: &Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(candidate)
        .map_err(|_| io::Error::other("state serialization failed"))?;
    let home = home.to_owned();
    let outcome = tokio::task::spawn_blocking(move || {
        let directory = match PrivateDirectory::open(&home, false) {
            Ok(d) => d,
            Err(e) => return CommitOutcome::NotCommitted(e),
        };
        directory.replace(OsStr::new(file), &bytes, create_only)
    })
    .await
    .unwrap_or_else(|_| {
        CommitOutcome::IndeterminateAfterRename(io::Error::other("storage worker failed"))
    });
    finish_outcome(outcome, state).await
}

async fn finish_outcome(outcome: CommitOutcome, state: &Arc<Mutex<DaemonState>>) -> io::Result<()> {
    match outcome {
        CommitOutcome::DurablyCommitted => Ok(()),
        CommitOutcome::NotCommitted(e) => Err(e),
        CommitOutcome::IndeterminateAfterRename(e) => {
            // Runtime state is explicitly degraded, not silently treated as a
            // rollback. No further administrative mutation may be dispatched.
            state.lock().await.storage_degraded = true;
            eprintln!("persistent state commit indeterminate; administrative mutations disabled; operator intervention required");
            Err(e)
        }
    }
}

pub(super) async fn handle(
    req: ControlRequest,
    state: Arc<Mutex<DaemonState>>,
    home: PathBuf,
) -> ControlResponse {
    if req.cmd == "pelt.init" {
        if state.lock().await.pelt.is_some() {
            return error(&req.id, "ALREADY_INITIALIZED");
        }
        let identity = generate_identity();
        if state_validation::identity(&identity).is_err() {
            return error(&req.id, "PELT_INVALID");
        }
        if durable(&home, "pelt.json", &identity, true, &state)
            .await
            .is_err()
        {
            return error(&req.id, "PELT_SAVE_FAILED");
        }
        let fingerprint = identity.fingerprint.clone();
        let mut live = state.lock().await;
        live.pelt = Some(identity);
        live.status.pelt_ready = true;
        return ControlResponse::ok(
            req.id,
            json!({"fingerprint":fingerprint,"saved_to":home.join("pelt.json")}),
        );
    }
    if req.cmd.starts_with("pack.") {
        let mut candidate = state.lock().await.peers.clone();
        let name = arg(&req, "name");
        let before = candidate.len();
        let status = match req.cmd.as_str() {
            "pack.add" => {
                candidate.push(PeerRecord {
                    name: name.clone(),
                    fingerprint: arg(&req, "fingerprint"),
                    address: arg(&req, "address"),
                    trust: TrustLevel::Packmate,
                    public_key_b64: None,
                });
                "added"
            }
            "pack.set_address" => {
                let Some(peer) = candidate.iter_mut().find(|p| p.name == name) else {
                    return error(&req.id, "PACK_NOT_FOUND");
                };
                peer.address = arg(&req, "address");
                "address_updated"
            }
            "pack.remove" | "pack.revoke" => {
                candidate.retain(|p| p.name != name);
                if candidate.len() == before {
                    return error(&req.id, "PACK_NOT_FOUND");
                }
                if req.cmd == "pack.revoke" {
                    "revoked"
                } else {
                    "removed"
                }
            }
            _ => return error(&req.id, "UNKNOWN_CMD"),
        };
        if state_validation::pack(&candidate).is_err() {
            return error(&req.id, "PACK_INVALID");
        }
        if durable(&home, "pack.json", &candidate, false, &state)
            .await
            .is_err()
        {
            return error(&req.id, "PACK_SAVE_FAILED");
        }
        let mut live = state.lock().await;
        live.peers = candidate;
        let closed = if req.cmd == "pack.revoke" {
            live.fang_registry.terminate_peer(&name)
        } else {
            0
        };
        return ControlResponse::ok(
            req.id,
            json!({"status":status,"name":name,"peer":name,"address":req.args["address"],"closed_fangs":closed,"packmates":live.peers.len()}),
        );
    }
    let (mut candidate, peers) = {
        let live = state.lock().await;
        (live.fang_profiles.clone(), live.peers.clone())
    };
    let name = arg(&req, "name");
    if req.cmd == "fang.profile.remove" && state.lock().await.active_profiles.contains(&name) {
        return error(&req.id, "FANG_PROFILE_ACTIVE");
    }
    let status = if req.cmd == "fang.profile.add" {
        candidate.push(FangProfile {
            name: name.clone(),
            peer: arg(&req, "peer"),
            local: arg(&req, "local"),
            remote: arg(&req, "remote"),
            transport: crate::policy::requested_transport(req.args["transport"].as_str()),
        });
        "profile_added"
    } else {
        let before = candidate.len();
        candidate.retain(|p| p.name != name);
        if before == candidate.len() {
            return error(&req.id, "FANG_PROFILE_NOT_FOUND");
        }
        "profile_removed"
    };
    if state_validation::profiles(&candidate, &peers).is_err() {
        return error(&req.id, "FANG_PROFILE_INVALID");
    }
    if durable(&home, "fangs.json", &candidate, false, &state)
        .await
        .is_err()
    {
        return error(&req.id, "FANG_PROFILE_SAVE_FAILED");
    }
    let mut live = state.lock().await;
    live.fang_profiles = candidate;
    ControlResponse::ok(
        req.id,
        json!({"status":status,"name":name,"profiles":live.fang_profiles.len()}),
    )
}

// Called only under the mutation coordinator (startup restoration does not write).
// The caller publishes registry changes immediately after this durable intent.
pub(crate) async fn persist_active(
    home: &Path,
    candidate: Vec<String>,
    state: &Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    {
        let live = state.lock().await;
        if live.storage_degraded {
            return Err(io::Error::other("storage degraded"));
        }
        state_validation::active(&candidate, &live.fang_profiles)?;
    }
    durable(home, "active_fangs.json", &candidate, false, state).await?;
    state.lock().await.active_profiles = candidate;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn indeterminate_commit_disables_subsequent_mutations() {
        let state = Arc::new(Mutex::new(DaemonState::default()));
        assert!(finish_outcome(
            CommitOutcome::NotCommitted(io::Error::other("injected")),
            &state
        )
        .await
        .is_err());
        assert!(!state.lock().await.storage_degraded);
        assert!(finish_outcome(
            CommitOutcome::IndeterminateAfterRename(io::Error::other("injected directory fsync")),
            &state
        )
        .await
        .is_err());
        assert!(state.lock().await.storage_degraded);
        let response = super::super::handle_request(
            ControlRequest {
                id: "test".into(),
                cmd: "pelt.init".into(),
                args: json!({}),
            },
            state,
            PathBuf::new(),
        )
        .await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "STORAGE_DEGRADED");
    }
}
