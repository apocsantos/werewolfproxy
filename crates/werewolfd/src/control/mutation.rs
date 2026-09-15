//! Control mutations enter through the single admission coordinator. Candidate
//! state is prepared off the live model, persisted off the Tokio worker, then
//! published without a fallible step. Client lifetime does not own this task.
use crate::{
    protected_state::{self, ProtectedState, Transaction},
    state::DaemonState,
    target_policy,
};
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
    local_fs::CommitOutcome,
    pack::{PeerRecord, TrustLevel},
    pelt::generate_identity,
    protocol::{ControlRequest, ControlResponse},
    state_validation,
};
use zeroize::Zeroizing;

pub(super) fn handles(command: &str) -> bool {
    matches!(
        command,
        "pelt.init"
            | "pack.add"
            | "pack.set_address"
            | "pack.revoke"
            | "pack.remove"
            | "target.policy.set"
            | "fang.profile.add"
            | "fang.profile.remove"
            | "state.manifest.migrate"
    )
}
fn error(id: &str, code: &str) -> ControlResponse {
    ControlResponse::err(id, code, "local state mutation rejected")
}
fn arg(req: &ControlRequest, field: &str) -> String {
    req.args[field].as_str().unwrap_or("").trim().to_owned()
}

async fn durable<T: Serialize + Sync>(
    _home: &Path,
    file: &'static str,
    candidate: &T,
    create_only: bool,
    state: &Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    // This coordinator also serializes Pelt, so own every temporary JSON
    // buffer with drop wiping on success, failure, or cancelled mutation.
    let bytes = Zeroizing::new(
        serde_json::to_vec_pretty(candidate)
            .map_err(|_| io::Error::other("state serialization failed"))?,
    );
    let directory = state
        .lock()
        .await
        .den
        .clone()
        .ok_or_else(|| io::Error::other("Den not initialized"))?;
    let outcome = tokio::task::spawn_blocking(move || {
        directory.replace(OsStr::new(file), &bytes, create_only)
    })
    .await
    .unwrap_or_else(|_| {
        CommitOutcome::IndeterminateAfterRename(io::Error::other("storage worker failed"))
    });
    finish_outcome(outcome, state).await
}

async fn durable_bytes(
    file: &'static str,
    bytes: Vec<u8>,
    state: &Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    let bytes = Zeroizing::new(bytes);
    let directory = state
        .lock()
        .await
        .den
        .clone()
        .ok_or_else(|| io::Error::other("Den not initialized"))?;
    let outcome =
        tokio::task::spawn_blocking(move || directory.replace(OsStr::new(file), &bytes, false))
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

fn protected_from_live(live: &DaemonState) -> ProtectedState {
    ProtectedState {
        silver_locked: live.inbound_authority.is_locked(),
        peers: live.peers.clone(),
        target_policy: live.target_policy.clone(),
        fang_profiles: live.fang_profiles.clone(),
        active_profiles: live.active_profiles.clone(),
    }
}

async fn commit_protected(
    generation: u64,
    candidate: ProtectedState,
    state: &Arc<Mutex<DaemonState>>,
) -> io::Result<u64> {
    let directory = state
        .lock()
        .await
        .den
        .clone()
        .ok_or_else(|| io::Error::other("Den not initialized"))?;
    let outcome = tokio::task::spawn_blocking(move || {
        protected_state::commit(&directory, generation, &candidate)
    })
    .await
    .unwrap_or_else(|_| {
        Transaction::IndeterminateAfterRename(io::Error::other("storage worker failed"))
    });
    match outcome {
        Transaction::DurablyCommitted(next) => {
            state.lock().await.protected_state_generation = Some(next.number);
            Ok(next.number)
        }
        Transaction::NotCommitted(error) => Err(error),
        Transaction::IndeterminateAfterRename(error) => {
            state.lock().await.storage_degraded = true;
            eprintln!("protected state commit indeterminate; administrative mutations disabled; operator intervention required");
            Err(error)
        }
    }
}

async fn persist_protected_or_legacy<T: Serialize + Sync>(
    generation: Option<u64>,
    candidate: ProtectedState,
    legacy_file: &'static str,
    legacy_value: &T,
    home: &Path,
    state: &Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    match generation {
        Some(generation) => {
            commit_protected(generation, candidate, state).await?;
            Ok(())
        }
        None => durable(home, legacy_file, legacy_value, false, state).await,
    }
}

pub(crate) async fn persist_silver(
    locked: bool,
    state: &Arc<Mutex<DaemonState>>,
) -> io::Result<()> {
    let (generation, mut candidate) = {
        let live = state.lock().await;
        (live.protected_state_generation, protected_from_live(&live))
    };
    candidate.silver_locked = locked;
    if let Some(generation) = generation {
        return commit_protected(generation, candidate, state)
            .await
            .map(|_| ());
    }
    let bytes: &'static [u8] = if locked {
        br#"{"version":1,"mode":"locked"}"#
    } else {
        br#"{"version":1,"mode":"open"}"#
    };
    let directory = state
        .lock()
        .await
        .den
        .clone()
        .ok_or_else(|| io::Error::other("Den not initialized"))?;
    let outcome = tokio::task::spawn_blocking(move || {
        directory.replace(OsStr::new("silver.json"), bytes, false)
    })
    .await
    .unwrap_or_else(|_| {
        CommitOutcome::IndeterminateAfterRename(io::Error::other("storage worker failed"))
    });
    finish_outcome(outcome, state).await
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
        // Construct and validate the ephemeral TLS identity before touching
        // durable state. Publication below installs both values atomically.
        let runtime_tls_identity =
            match crate::tls_identity::RuntimeTlsIdentity::from_pelt(&identity) {
                Ok(identity) => Arc::new(identity),
                Err(_) => return error(&req.id, "PELT_TLS_INVALID"),
            };
        if durable(&home, "pelt.json", &identity, true, &state)
            .await
            .is_err()
        {
            return error(&req.id, "PELT_SAVE_FAILED");
        }
        let fingerprint = identity.fingerprint.clone();
        let mut live = state.lock().await;
        live.pelt = Some(identity);
        live.runtime_tls_identity = Some(runtime_tls_identity);
        live.status.pelt_ready = true;
        // Only signal after durable persistence and coherent state publication.
        // The state lock prevents a listener from seeing READY before identity.
        live.tls_identity_ready.publish();
        return ControlResponse::ok(
            req.id,
            json!({"fingerprint":fingerprint,"saved_to":home.join("pelt.json")}),
        );
    }
    if req.cmd == "state.manifest.migrate" {
        let directory = {
            let live = state.lock().await;
            if live.protected_state_generation.is_some() {
                return error(&req.id, "MANIFEST_ALREADY_ENABLED");
            }
            match &live.den {
                Some(directory) => directory.clone(),
                None => return error(&req.id, "MANIFEST_MIGRATION_FAILED"),
            }
        };
        let outcome = tokio::task::spawn_blocking(move || protected_state::migrate(&directory))
            .await
            .unwrap_or_else(|_| {
                Transaction::IndeterminateAfterRename(io::Error::other("storage worker failed"))
            });
        match outcome {
            Transaction::DurablyCommitted(generation) => {
                state.lock().await.protected_state_generation = Some(generation.number);
                return ControlResponse::ok(req.id, json!({"generation": generation.number}));
            }
            Transaction::NotCommitted(_) => return error(&req.id, "MANIFEST_MIGRATION_FAILED"),
            Transaction::IndeterminateAfterRename(_) => {
                state.lock().await.storage_degraded = true;
                return error(&req.id, "MANIFEST_MIGRATION_INDETERMINATE");
            }
        }
    }
    if req.cmd == "target.policy.set" {
        let document = arg(&req, "document");
        let canonical = match target_policy::canonicalize(&document) {
            Ok(canonical) => canonical,
            Err(_) => return error(&req.id, "TARGET_POLICY_INVALID"),
        };
        let parsed = match target_policy::parse_canonical(&canonical) {
            Ok(policy) => policy,
            Err(_) => return error(&req.id, "TARGET_POLICY_INVALID"),
        };
        let (generation, mut candidate) = {
            let live = state.lock().await;
            (live.protected_state_generation, protected_from_live(&live))
        };
        candidate.target_policy = parsed.clone();
        let persisted = match generation {
            Some(generation) => commit_protected(generation, candidate, &state)
                .await
                .map(|_| ()),
            None => durable_bytes("target_policy.json", canonical, &state).await,
        };
        if persisted.is_err() {
            return error(&req.id, "TARGET_POLICY_SAVE_FAILED");
        }
        state.lock().await.target_policy = parsed;
        return ControlResponse::ok(req.id, json!({"status":"target_policy_updated"}));
    }
    if req.cmd.starts_with("pack.") {
        let (mut candidate, protected_generation) = {
            let live = state.lock().await;
            (live.peers.clone(), live.protected_state_generation)
        };
        let name = arg(&req, "name");
        let before = candidate.len();
        let revoked_fingerprint = if matches!(req.cmd.as_str(), "pack.remove" | "pack.revoke") {
            candidate
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.fingerprint.clone())
        } else {
            None
        };
        let status = match req.cmd.as_str() {
            "pack.add" => {
                candidate.push(PeerRecord {
                    name: name.clone(),
                    fingerprint: arg(&req, "fingerprint"),
                    address: arg(&req, "address"),
                    trust: TrustLevel::Packmate,
                    public_key_b64: {
                        let value = arg(&req, "public_key_b64");
                        (!value.is_empty()).then_some(value)
                    },
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
        let (authority, closed) = {
            let mut live = state.lock().await;
            let authority = live.inbound_authority.clone();
            let closed = if let Some(peer) = &revoked_fingerprint {
                // Authority linearizes before filesystem work. The coordinator
                // owns this operation after acceptance, not the client's socket.
                if authority.deny_peer(peer).is_err() {
                    return error(&req.id, "AUTHORITY_UNAVAILABLE");
                }
                let closed = live.fang_registry.terminate_peer(&name);
                live.status.active_fangs = live.fang_registry.len();
                closed
            } else {
                0
            };
            (authority, closed)
        };
        let persisted = if let Some(generation) = protected_generation {
            let mut protected = {
                let live = state.lock().await;
                protected_from_live(&live)
            };
            protected.peers = candidate.clone();
            commit_protected(generation, protected, &state)
                .await
                .map(|_| ())
        } else {
            durable(&home, "pack.json", &candidate, false, &state).await
        };
        if persisted.is_err() {
            // NotCommitted leaves Pack memory/disk unchanged but never restores
            // runtime authority. Indeterminate also retains the deny fence and
            // durable() enters the existing storage-degraded path.
            return error(
                &req.id,
                if revoked_fingerprint.is_some() {
                    "PACK_SAVE_FAILED_RUNTIME_DENIED"
                } else {
                    "PACK_SAVE_FAILED"
                },
            );
        }
        let mut live = state.lock().await;
        live.peers = candidate;
        if let Some(peer) = &revoked_fingerprint {
            if authority.removed(peer).is_err() {
                // Disk is committed; never pretend rollback or return success
                // when matching authority publication cannot be established.
                live.storage_degraded = true;
                return error(&req.id, "AUTHORITY_PUBLICATION_FAILED");
            }
        }
        let packmates = live.peers.len();
        drop(live);
        if let Some(peer) = &revoked_fingerprint {
            if authority.cleanup(Some(peer)).await.is_err() {
                return error(&req.id, "AUTHORITY_CLEANUP_TIMEOUT");
            }
        }
        return ControlResponse::ok(
            req.id,
            json!({"status":status,"name":name,"peer":name,"address":req.args["address"],"closed_fangs":closed,"packmates":packmates}),
        );
    }
    let (mut candidate, peers, protected_generation) = {
        let live = state.lock().await;
        (
            live.fang_profiles.clone(),
            live.peers.clone(),
            live.protected_state_generation,
        )
    };
    let name = arg(&req, "name");
    if req.cmd == "fang.profile.remove" && state.lock().await.active_profiles.contains(&name) {
        return error(&req.id, "FANG_PROFILE_ACTIVE");
    }
    let status = if req.cmd == "fang.profile.add" {
        if !peers.iter().any(|peer| peer.name == arg(&req, "peer")) {
            return error(&req.id, "FANG_UNKNOWN_PEER");
        }
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
    if state_validation::profile_document(&candidate).is_err() {
        return error(&req.id, "FANG_PROFILE_INVALID");
    }
    let persisted = if let Some(generation) = protected_generation {
        let mut protected = {
            let live = state.lock().await;
            protected_from_live(&live)
        };
        protected.fang_profiles = candidate.clone();
        commit_protected(generation, protected, &state)
            .await
            .map(|_| ())
    } else {
        durable(&home, "fangs.json", &candidate, false, &state).await
    };
    if persisted.is_err() {
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
    let (generation, mut protected) = {
        let live = state.lock().await;
        (live.protected_state_generation, protected_from_live(&live))
    };
    protected.active_profiles = candidate.clone();
    persist_protected_or_legacy(
        generation,
        protected,
        "active_fangs.json",
        &candidate,
        home,
        state,
    )
    .await?;
    state.lock().await.active_profiles = candidate;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    use werewolf_core::{
        local_fs::{CommitOutcome, PrivateDirectory},
        pelt::generate_identity,
    };

    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "wwp-manifest-mutation-{}",
                crate::handshake::hex(&crate::handshake::random::<16>().unwrap())
            ));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }

        fn state(&self) -> Arc<Mutex<DaemonState>> {
            Arc::new(Mutex::new(DaemonState {
                den: Some(Arc::new(PrivateDirectory::open(&self.0, false).unwrap())),
                ..DaemonState::default()
            }))
        }

        fn state_with_silver_open(&self) -> Arc<Mutex<DaemonState>> {
            Arc::new(Mutex::new(DaemonState {
                den: Some(Arc::new(PrivateDirectory::open(&self.0, false).unwrap())),
                inbound_authority: crate::authority::Authority::new(false),
                ..DaemonState::default()
            }))
        }

        fn write(&self, name: &str, bytes: &[u8]) {
            let directory = PrivateDirectory::open(&self.0, false).unwrap();
            assert!(matches!(
                directory.replace(OsStr::new(name), bytes, false),
                CommitOutcome::DurablyCommitted
            ));
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

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
        assert!(!state.lock().await.tls_identity_ready.is_ready());
        assert!(finish_outcome(
            CommitOutcome::IndeterminateAfterRename(io::Error::other("injected directory fsync")),
            &state
        )
        .await
        .is_err());
        assert!(state.lock().await.storage_degraded);
        assert!(!state.lock().await.tls_identity_ready.is_ready());
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

    #[tokio::test]
    async fn manifest_mode_routes_every_protected_mutation_through_one_generation() {
        let fixture = Fixture::new();
        let state = fixture.state();
        let migrate = handle(
            ControlRequest {
                id: "migrate".into(),
                cmd: "state.manifest.migrate".into(),
                args: json!({}),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(migrate.ok);
        assert_eq!(state.lock().await.protected_state_generation, Some(1));

        let peer = generate_identity();
        let add = handle(
            ControlRequest {
                id: "pack".into(),
                cmd: "pack.add".into(),
                args: json!({
                    "name":"peer", "fingerprint":peer.fingerprint,
                    "address":"tcp://127.0.0.1:1", "public_key_b64":peer.public_key_b64,
                }),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(add.ok);
        assert_eq!(state.lock().await.protected_state_generation, Some(2));
        assert!(!fixture.0.join("pack.json").exists());

        let policy = serde_json::json!({
            "mode":"deny-by-default",
            "peers": { peer.fingerprint.clone(): {"targets":[{"address":"127.0.0.1","port":1}]} }
        })
        .to_string();
        let policy = handle(
            ControlRequest {
                id: "policy".into(),
                cmd: "target.policy.set".into(),
                args: json!({"document":policy}),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(policy.ok);
        assert_eq!(state.lock().await.protected_state_generation, Some(3));
        assert!(!fixture.0.join("target_policy.json").exists());

        let profile = handle(
            ControlRequest {
                id: "profile".into(),
                cmd: "fang.profile.add".into(),
                args: json!({
                    "name":"route", "peer":"peer", "local":"127.0.0.1:1",
                    "remote":"127.0.0.1:2", "transport":"tcp"
                }),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(profile.ok);
        assert_eq!(state.lock().await.protected_state_generation, Some(4));
        persist_active(&fixture.0, vec!["route".into()], &state)
            .await
            .unwrap();
        assert_eq!(state.lock().await.protected_state_generation, Some(5));
        assert!(!fixture.0.join("fangs.json").exists());
        assert!(!fixture.0.join("active_fangs.json").exists());
    }

    #[tokio::test]
    async fn silver_fences_before_manifest_commit_and_opens_only_after_it() {
        let fixture = Fixture::new();
        fixture.write("silver.json", br#"{"version":1,"mode":"open"}"#);
        let state = fixture.state_with_silver_open();
        assert!(
            handle(
                ControlRequest {
                    id: "migrate".into(),
                    cmd: "state.manifest.migrate".into(),
                    args: json!({})
                },
                state.clone(),
                fixture.0.clone()
            )
            .await
            .ok
        );
        assert!(
            super::super::handle_request(
                ControlRequest {
                    id: "on".into(),
                    cmd: "silver.trigger".into(),
                    args: json!({})
                },
                state.clone(),
                fixture.0.clone()
            )
            .await
            .ok
        );
        assert!(state.lock().await.inbound_authority.is_locked());
        assert_eq!(state.lock().await.protected_state_generation, Some(2));
        assert!(matches!(
            protected_state::load(&PrivateDirectory::open(&fixture.0, false).unwrap()).unwrap(),
            protected_state::Load::Manifest { state, .. } if state.silver_locked
        ));
        assert!(
            super::super::handle_request(
                ControlRequest {
                    id: "off".into(),
                    cmd: "silver.reset".into(),
                    args: json!({})
                },
                state.clone(),
                fixture.0.clone()
            )
            .await
            .ok
        );
        assert!(!state.lock().await.inbound_authority.is_locked());
        assert_eq!(state.lock().await.protected_state_generation, Some(3));
    }
}
