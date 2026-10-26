//! Control mutations enter through the single admission coordinator. Candidate
//! state is prepared off the live model, persisted off the Tokio worker, then
//! published without a fallible step. Client lifetime does not own this task.
use crate::{
    fang_registry::FangTermination,
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
    service::{ServiceDefinition, ServiceKind},
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
            | "target.allow"
            | "target.remove"
            | "fang.profile.add"
            | "fang.profile.remove"
            | "service.add"
            | "service.remove"
            | "service.publish"
            | "service.unpublish"
            | "service.provider.add"
            | "service.provider.remove"
            | "governance.apply"
            | "governance.sync_apply"
            | "den.retire"
            | "route.set"
            | "route.remove"
            | "discover.advertise"
            | "state.manifest.migrate"
    )
}

/// Writes the non-authoritative discovery cache through the same durable
/// path as other Den files. Called outside the mutation coordinator: the
/// cache is serialized by the discovery lock and grants nothing.
pub(crate) async fn persist_discovery_cache(
    state: &Arc<Mutex<DaemonState>>,
    cache: &crate::discovery::operator::CacheFile,
) -> io::Result<()> {
    durable(
        Path::new(""),
        crate::discovery::operator::CACHE_FILE,
        cache,
        false,
        state,
    )
    .await
}
fn error(id: &str, code: &str) -> ControlResponse {
    ControlResponse::err(id, code, "local state mutation rejected")
}
fn service_kind(kind: &str) -> Option<ServiceKind> {
    Some(match kind {
        "tcp" => ServiceKind::Tcp,
        "http" => ServiceKind::Http,
        "https" => ServiceKind::Https,
        "ssh" => ServiceKind::Ssh,
        _ => return None,
    })
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
    #[cfg(test)]
    let outcome = if file == "pack.json"
        && state.lock().await.pack_commit_indeterminate
        && matches!(outcome, CommitOutcome::DurablyCommitted)
    {
        CommitOutcome::IndeterminateAfterRename(io::Error::other("injected after Pack rename"))
    } else {
        outcome
    };
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
        governance: live.governance.clone(),
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
    if req.cmd == "den.retire" {
        let confirm = req.args["confirm"].as_str().unwrap_or("").to_owned();
        return match crate::terminal::retire(&state, &confirm).await {
            Ok(result) => ControlResponse::ok(&req.id, result),
            Err((code, message)) => ControlResponse::err(&req.id, code, message),
        };
    }
    if req.cmd == "pelt.init" {
        if state.lock().await.pelt.is_some() {
            return error(&req.id, "ALREADY_INITIALIZED");
        }
        // Stage30: a Den that holds a Tombstone is never initialized again.
        if let Some(den) = state.lock().await.den.clone() {
            if !matches!(
                den.read(
                    std::ffi::OsStr::new(werewolf_core::tombstone::FILE_NAME),
                    werewolf_core::tombstone::MAX_LEN + 1,
                ),
                Ok(None)
            ) {
                return error(&req.id, "LIFECYCLE_TOMBSTONE");
            }
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
    if matches!(
        req.cmd.as_str(),
        "target.policy.set" | "target.allow" | "target.remove"
    ) {
        let parsed = if req.cmd == "target.policy.set" {
            let document = arg(&req, "document");
            let canonical = match target_policy::canonicalize(&document) {
                Ok(canonical) => canonical,
                Err(_) => return error(&req.id, "TARGET_POLICY_INVALID"),
            };
            match target_policy::parse_canonical(&canonical) {
                Ok(policy) => policy,
                Err(_) => return error(&req.id, "TARGET_POLICY_INVALID"),
            }
        } else {
            let peer = arg(&req, "peer");
            let target = match arg(&req, "target").parse() {
                Ok(target) => target,
                Err(_) => return error(&req.id, "TARGET_INVALID"),
            };
            let (fingerprint, current) = {
                let live = state.lock().await;
                let fingerprint = live
                    .peers
                    .iter()
                    .find(|record| record.name == peer || record.fingerprint == peer)
                    .map(|record| record.fingerprint.clone());
                (fingerprint, live.target_policy.clone())
            };
            let Some(fingerprint) = fingerprint else {
                return error(&req.id, "PACK_NOT_FOUND");
            };
            let result = if req.cmd == "target.allow" {
                target_policy::allow_exact(&current, &fingerprint, target)
            } else {
                target_policy::remove_exact(&current, &fingerprint, target)
            };
            match result {
                Ok(policy) => policy,
                Err(_) if req.cmd == "target.remove" => return error(&req.id, "TARGET_NOT_FOUND"),
                Err(_) => return error(&req.id, "TARGET_POLICY_INVALID"),
            }
        };
        let canonical = match target_policy::canonical_from_runtime(&parsed) {
            Ok(canonical) => canonical,
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
        return ControlResponse::ok(
            req.id,
            json!({"status": if req.cmd == "target.allow" { "target_allowed" } else if req.cmd == "target.remove" { "target_removed" } else { "target_policy_updated" }}),
        );
    }
    if matches!(req.cmd.as_str(), "service.add" | "service.remove") {
        let mut candidate = {
            let live = state.lock().await;
            live.services.clone()
        };

        let name = arg(&req, "name");
        if name.is_empty() {
            return error(&req.id, "SERVICE_INVALID");
        }

        let status = if req.cmd == "service.add" {
            if candidate.iter().any(|service| service.name == name) {
                return error(&req.id, "SERVICE_EXISTS");
            }

            let Some(kind) = service_kind(&arg(&req, "kind")) else {
                return error(&req.id, "SERVICE_INVALID");
            };

            candidate.push(ServiceDefinition {
                name: name.clone(),
                peer: arg(&req, "peer"),
                target: arg(&req, "target"),
                kind,
                fallbacks: Vec::new(),
            });

            "added"
        } else {
            let before = candidate.len();
            candidate.retain(|service| service.name != name);

            if candidate.len() == before {
                return error(&req.id, "SERVICE_NOT_FOUND");
            }

            "removed"
        };

        if werewolf_core::service::validate_services(&candidate).is_err() {
            return error(&req.id, "SERVICE_INVALID");
        }

        if durable(&home, "services.json", &candidate, false, &state)
            .await
            .is_err()
        {
            return error(&req.id, "SERVICE_SAVE_FAILED");
        }

        let mut live = state.lock().await;
        live.services = candidate;

        return ControlResponse::ok(
            req.id,
            json!({
                "status": status,
                "name": name,
                "services": live.services.len()
            }),
        );
    }

    if matches!(req.cmd.as_str(), "service.publish" | "service.unpublish") {
        // Publishing announces a service to peers already granted its
        // target; it grants nothing (Stage25 decision 2).
        let mut candidate = state.lock().await.published.clone();
        let name = arg(&req, "name");
        if name.is_empty() {
            return error(&req.id, "SERVICE_INVALID");
        }
        let status = if req.cmd == "service.publish" {
            if candidate.iter().any(|service| service.name == name) {
                return error(&req.id, "SERVICE_EXISTS");
            }
            let Some(kind) = service_kind(&arg(&req, "kind")) else {
                return error(&req.id, "SERVICE_INVALID");
            };
            candidate.push(werewolf_core::published::PublishedService {
                name: name.clone(),
                kind,
                target: arg(&req, "target"),
            });
            "published"
        } else {
            let before = candidate.len();
            candidate.retain(|service| service.name != name);
            if candidate.len() == before {
                return error(&req.id, "SERVICE_NOT_FOUND");
            }
            "unpublished"
        };
        if werewolf_core::published::validate_published(&candidate).is_err() {
            return error(&req.id, "SERVICE_INVALID");
        }
        if durable(
            &home,
            crate::discovery::operator::PUBLISHED_FILE,
            &candidate,
            false,
            &state,
        )
        .await
        .is_err()
        {
            return error(&req.id, "SERVICE_SAVE_FAILED");
        }
        let mut live = state.lock().await;
        live.published = candidate;
        return ControlResponse::ok(
            req.id,
            json!({"status": status, "name": name, "published": live.published.len()}),
        );
    }

    if matches!(
        req.cmd.as_str(),
        "service.provider.add" | "service.provider.remove"
    ) {
        // Stage27: providers are tried in this order per connection; each
        // still authorizes the requester itself. The list is intent only.
        let mut candidate = state.lock().await.services.clone();
        let name = arg(&req, "name");
        let peer = arg(&req, "peer");
        let Some(service) = candidate.iter_mut().find(|service| service.name == name) else {
            return error(&req.id, "SERVICE_NOT_FOUND");
        };
        let status = if req.cmd == "service.provider.add" {
            // From `--from`, the provider's own kind label must match; a
            // different kind under the same name is a different service.
            let kind = arg(&req, "kind");
            if !kind.is_empty() && service_kind(&kind) != Some(service.kind.clone()) {
                return error(&req.id, "SERVICE_INVALID");
            }
            service
                .fallbacks
                .push(werewolf_core::service::ServiceProvider {
                    peer: peer.clone(),
                    target: arg(&req, "target"),
                });
            "provider_added"
        } else if service.peer == peer {
            // Removing the first provider promotes the next one; the last
            // provider goes only with `service remove`.
            if service.fallbacks.is_empty() {
                return error(&req.id, "SERVICE_INVALID");
            }
            let next = service.fallbacks.remove(0);
            service.peer = next.peer;
            service.target = next.target;
            "provider_removed"
        } else {
            let before = service.fallbacks.len();
            service.fallbacks.retain(|provider| provider.peer != peer);
            if service.fallbacks.len() == before {
                return error(&req.id, "SERVICE_NOT_FOUND");
            }
            "provider_removed"
        };
        let providers = service.fallbacks.len() + 1;
        if werewolf_core::service::validate_services(&candidate).is_err() {
            return error(&req.id, "SERVICE_INVALID");
        }
        if durable(&home, "services.json", &candidate, false, &state)
            .await
            .is_err()
        {
            return error(&req.id, "SERVICE_SAVE_FAILED");
        }
        state.lock().await.services = candidate;
        return ControlResponse::ok(
            req.id,
            json!({"status": status, "name": name, "peer": peer, "providers": providers}),
        );
    }

    if matches!(req.cmd.as_str(), "route.set" | "route.remove") {
        // A route changes how bytes reach a Pack member, never who it is or
        // what it allows (Stage26 decision 1A); it grants nothing.
        let (mut candidate, peers) = {
            let st = state.lock().await;
            (st.routes.clone(), st.peers.clone())
        };
        let peer = arg(&req, "peer");
        let status = if req.cmd == "route.set" {
            let via = arg(&req, "via");
            let hop = match arg(&req, "hop").as_str() {
                "" => Some(werewolf_core::route::RouteHop::Quic),
                hop => werewolf_core::route::RouteHop::parse(hop),
            };
            let Some(hop) = hop else {
                return error(&req.id, "ROUTE_INVALID");
            };
            // Both ends must be Pack members now; use time checks again.
            if ![&peer, &via]
                .iter()
                .all(|name| peers.iter().any(|member| &&member.name == name))
            {
                return error(&req.id, "ROUTE_INVALID");
            }
            let route = werewolf_core::route::Route {
                peer: peer.clone(),
                via,
                hop,
            };
            match candidate.iter_mut().find(|existing| existing.peer == peer) {
                Some(existing) => {
                    *existing = route;
                    "updated"
                }
                None => {
                    candidate.push(route);
                    "set"
                }
            }
        } else {
            let before = candidate.len();
            candidate.retain(|route| route.peer != peer);
            if candidate.len() == before {
                return error(&req.id, "ROUTE_NOT_FOUND");
            }
            "removed"
        };
        if werewolf_core::route::validate_routes(&candidate).is_err() {
            return error(&req.id, "ROUTE_INVALID");
        }
        if durable(
            &home,
            crate::persistence::ROUTES_FILE,
            &candidate,
            false,
            &state,
        )
        .await
        .is_err()
        {
            return error(&req.id, "ROUTE_SAVE_FAILED");
        }
        let mut live = state.lock().await;
        live.routes = candidate;
        return ControlResponse::ok(
            req.id,
            json!({"status": status, "peer": peer, "routes": live.routes.len()}),
        );
    }

    if req.cmd == "discover.advertise" {
        // Advertising publishes reachability only; it grants no access.
        // Control arguments are strings: comma-separated, empty withdraws.
        let addresses: Vec<String> = arg(&req, "addresses")
            .split(',')
            .map(|address| address.trim().to_owned())
            .filter(|address| !address.is_empty())
            .collect();
        if !addresses.is_empty() && werewolf_core::contact::check_addresses(&addresses).is_err() {
            return error(&req.id, "DISCOVER_ADDRESS_INVALID");
        }
        let file = crate::discovery::operator::AdvertiseFile {
            version: 1,
            addresses: addresses.clone(),
        };
        if durable(
            &home,
            crate::discovery::operator::ADVERTISE_FILE,
            &file,
            false,
            &state,
        )
        .await
        .is_err()
        {
            return error(&req.id, "DISCOVER_SAVE_FAILED");
        }
        let mut live = state.lock().await;
        live.contacts.advertised = addresses;
        if let Some(pelt) = live.pelt.clone() {
            live.contacts
                .refresh_own(&pelt, crate::discovery::unix_now());
        }
        let record = live
            .contacts
            .own
            .as_ref()
            .map(|record| crate::discovery::operator::view(record, &live.peers));
        return ControlResponse::ok(
            req.id,
            json!({"advertised": live.contacts.advertised, "record": record}),
        );
    }

    if req.cmd == "governance.apply" {
        return apply_governance(req, home, state).await;
    }
    if req.cmd == "governance.sync_apply" {
        return apply_sync(req, state).await;
    }

    if req.cmd.starts_with("pack.") {
        let (mut candidate, protected_generation) = {
            let live = state.lock().await;
            (live.peers.clone(), live.protected_state_generation)
        };
        let name = arg(&req, "name");
        let before = candidate.len();
        let mut address_changed = false;
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
                let public_key_b64 = arg(&req, "public_key_b64");
                if public_key_b64.is_empty()
                    || werewolf_core::pelt::fingerprint_from_public_key_b64(&public_key_b64)
                        .ok()
                        .as_deref()
                        != Some(arg(&req, "fingerprint").as_str())
                {
                    return error(&req.id, "PACK_PUBLIC_KEY_INVALID");
                }
                // Stage28: local administration never re-trusts a member the
                // network revoked.
                let key = werewolf_core::governance::tool::public_key(&public_key_b64).ok();
                let (revoked, off_roster) = match (&state.lock().await.governance, key) {
                    (Some(head), Some(key)) => (
                        head.revoked.binary_search(&key).is_ok(),
                        head.activated_at.is_some() && head.roster.binary_search(&key).is_err(),
                    ),
                    _ => (false, false),
                };
                if revoked {
                    return ControlResponse::err(
                        &req.id,
                        "PACK_REVOKED_BY_NETWORK",
                        "this key was revoked by a governance statement",
                    );
                }
                // Stage29 4a: after activation the roster bounds the Pack.
                if off_roster {
                    return ControlResponse::err(
                        &req.id,
                        "PACK_NOT_ON_ROSTER",
                        "this key is not on the activated network's roster",
                    );
                }
                candidate.push(PeerRecord {
                    name: name.clone(),
                    fingerprint: arg(&req, "fingerprint"),
                    address: arg(&req, "address"),
                    trust: TrustLevel::Packmate,
                    public_key_b64: Some(public_key_b64),
                });
                "added"
            }
            "pack.set_address" => {
                let Some(peer) = candidate.iter_mut().find(|p| p.name == name) else {
                    return error(&req.id, "PACK_NOT_FOUND");
                };
                let address = arg(&req, "address");
                address_changed = peer.address != address;
                peer.address = address;
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
        if req.cmd == "pack.set_address" {
            let closed = match apply_pack_address_change(
                &home,
                &name,
                candidate.clone(),
                address_changed,
                protected_generation,
                &state,
            )
            .await
            {
                Ok(closed) => closed,
                Err(_) => return error(&req.id, "PACK_SAVE_FAILED"),
            };
            let packmates = state.lock().await.peers.len();
            return ControlResponse::ok(
                req.id,
                json!({"status":status,"name":name,"peer":name,"address":req.args["address"],"closed_fangs":closed,"packmates":packmates}),
            );
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

async fn apply_pack_address_change(
    home: &Path,
    peer_name: &str,
    candidate_peers: Vec<PeerRecord>,
    address_changed: bool,
    generation: Option<u64>,
    state: &Arc<Mutex<DaemonState>>,
) -> io::Result<usize> {
    let (old_active, candidate_active, affected_ids) = {
        let live = state.lock().await;
        let old_active = live.active_profiles.clone();
        let mut candidate_active = old_active.clone();
        let affected_ids = if address_changed {
            // Profiles to this peer, or routed through it as the relay
            // (Stage26), need explicit reactivation, as do their Fangs.
            let uses_peer = |profile_peer: &str| {
                profile_peer == peer_name
                    || live
                        .routes
                        .iter()
                        .any(|route| route.peer == profile_peer && route.via == peer_name)
            };
            candidate_active.retain(|profile_name| {
                !live
                    .fang_profiles
                    .iter()
                    .any(|profile| profile.name == *profile_name && uses_peer(&profile.peer))
            });
            live.fang_registry.ids_using_peer(peer_name)
        } else {
            Vec::new()
        };
        (old_active, candidate_active, affected_ids)
    };

    {
        let live = state.lock().await;
        state_validation::active(&candidate_active, &live.fang_profiles)?;
    }

    if let Some(generation) = generation {
        let mut protected = {
            let live = state.lock().await;
            protected_from_live(&live)
        };
        protected.peers = candidate_peers.clone();
        protected.active_profiles = candidate_active.clone();
        if let Err(error) = commit_protected(generation, protected, state).await {
            if state.lock().await.storage_degraded {
                let termination = {
                    let mut live = state.lock().await;
                    publish_invalidated_runtime(&mut live, &candidate_active, &affected_ids)
                };
                termination.quiesce().await;
            }
            return Err(error);
        }
    } else {
        // Legacy documents do not have the protected-state generation's
        // single commit boundary. Publish the fail-closed active intent first;
        // if Pack persistence is NotCommitted, restore the old intent through
        // the same durable machinery. An indeterminate Pack commit leaves the
        // existing degraded state untouched and never attempts rollback.
        if candidate_active != old_active {
            if let Err(error) =
                durable(home, "active_fangs.json", &candidate_active, false, state).await
            {
                if state.lock().await.storage_degraded {
                    let termination = {
                        let mut live = state.lock().await;
                        publish_invalidated_runtime(&mut live, &candidate_active, &affected_ids)
                    };
                    termination.quiesce().await;
                }
                return Err(error);
            }
        }
        if let Err(error) = durable(home, "pack.json", &candidate_peers, false, state).await {
            if !state.lock().await.storage_degraded {
                if durable(home, "active_fangs.json", &old_active, false, state)
                    .await
                    .is_err()
                {
                    state.lock().await.storage_degraded = true;
                    let termination = {
                        let mut live = state.lock().await;
                        publish_invalidated_runtime(&mut live, &candidate_active, &affected_ids)
                    };
                    termination.quiesce().await;
                }
            } else {
                let termination = {
                    let mut live = state.lock().await;
                    publish_invalidated_runtime(&mut live, &candidate_active, &affected_ids)
                };
                termination.quiesce().await;
            }
            return Err(error);
        }
    }

    let termination = {
        let mut live = state.lock().await;
        live.peers = candidate_peers;
        publish_invalidated_runtime(&mut live, &candidate_active, &affected_ids)
    };
    let closed = termination.closed();
    termination.quiesce().await;
    Ok(closed)
}

/// Stage28 `governance.apply`: decode the statement and apply it with
/// `apply_statement`.
async fn apply_governance(
    req: ControlRequest,
    _home: PathBuf,
    state: Arc<Mutex<DaemonState>>,
) -> ControlResponse {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use werewolf_core::governance::{tool, Statement};
    let reject = |code: &str, message: String| ControlResponse::err(&req.id, code, message);
    let Ok(bytes) = STANDARD.decode(arg(&req, "statement")) else {
        return reject("GOVERNANCE_INVALID", "statement is not base64".into());
    };
    let statement = match Statement::decode(&bytes) {
        Ok(statement) => statement,
        Err(error) => return reject("GOVERNANCE_INVALID", error.to_string()),
    };
    let expect = match arg(&req, "expect_hash").as_str() {
        "" => None,
        hex => match tool::unhex::<32>(hex) {
            Ok(hash) => Some(hash),
            Err(_) => {
                return reject(
                    "GOVERNANCE_INVALID",
                    "expect_hash is not 64 hex digits".into(),
                )
            }
        },
    };
    match apply_statement(&statement, expect.as_ref(), &state).await {
        Ok(result) => ControlResponse::ok(req.id, result),
        Err((code, message)) => reject(code, message),
    }
}

/// Stage29 `governance.sync_apply`, the mutation half of `governance sync`:
/// the statements the network phase found, in order, each re-verified and
/// applied against the live head exactly as `governance apply`, stopping at
/// the first that does not apply; or a conflict of two valid successors,
/// recorded as a fork before either is applied (addition D). Internal: the
/// control socket refuses it from clients.
async fn apply_sync(req: ControlRequest, state: Arc<Mutex<DaemonState>>) -> ControlResponse {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use werewolf_core::governance::Statement;
    let decode = |value: &serde_json::Value| {
        value
            .as_str()
            .and_then(|text| STANDARD.decode(text).ok())
            .and_then(|bytes| Statement::decode(&bytes).ok())
    };
    let peers = req.args["peers"].clone();
    let conflict: Option<Vec<Statement>> = req.args["conflict"]
        .as_array()
        .map(|pair| pair.iter().filter_map(decode).collect());
    let Some(statements) = req.args["statements"]
        .as_array()
        .map(|list| list.iter().map(decode).collect::<Option<Vec<_>>>())
        .unwrap_or(Some(Vec::new()))
    else {
        return ControlResponse::err(&req.id, "GOVERNANCE_INVALID", "malformed sync request");
    };
    let mut applied = Vec::new();
    for statement in &statements {
        match apply_statement(statement, None, &state).await {
            Ok(result) => applied.push(result),
            Err((code, message)) => {
                return ControlResponse::ok(
                    req.id,
                    json!({"peers": peers, "applied": applied, "stopped": {"code": code, "message": message}}),
                )
            }
        }
    }
    if let Some(pair) = conflict {
        let (code, message) = record_conflict(&pair, &state).await;
        return ControlResponse::ok(
            req.id,
            json!({"peers": peers, "applied": applied, "stopped": {"code": code, "message": message}}),
        );
    }
    ControlResponse::ok(req.id, json!({"peers": peers, "applied": applied}))
}

/// Two different statements, both valid successors of the live head: the
/// threshold signed two histories. Record the fork durably before applying
/// either (addition D). If they are no longer both valid successors (the
/// head moved), nothing is recorded.
async fn record_conflict(
    pair: &[werewolf_core::governance::Statement],
    state: &Arc<Mutex<DaemonState>>,
) -> (&'static str, String) {
    use werewolf_core::governance::{self, Outcome};
    let (head, generation) = {
        let live = state.lock().await;
        (live.governance.clone(), live.protected_state_generation)
    };
    let (Some(head), Some(generation), [a, b]) = (head, generation, pair) else {
        return ("GOVERNANCE_REJECTED", "conflict no longer applies".into());
    };
    let valid = |s| {
        matches!(
            governance::apply(Some(&head), s, None),
            Ok(Outcome::Applied(_))
        )
    };
    if a.core.hash() == b.core.hash() || !valid(a) || !valid(b) {
        return ("GOVERNANCE_REJECTED", "conflict no longer applies".into());
    }
    let mut fenced = head.clone();
    fenced.forked = true;
    let mut protected = protected_from_live(&*state.lock().await);
    protected.governance = Some(fenced.clone());
    if commit_protected(generation, protected, state).await.is_ok() {
        state.lock().await.governance = Some(fenced);
    }
    (
        "GOVERNANCE_FORK",
        format!(
            "peers offered two different valid statements for sequence {}; governance is fenced",
            a.core.sequence
        ),
    )
}

/// Verify `statement` against the head and, if it is the next one, commit
/// the new head (and, for a revocation or activation, the Pack without the
/// members it excludes) in one protected generation. The statement is
/// archived first (G3): the archive is part of the apply, a failed archive
/// write fails it with no effect, and the Den's atomic replace reuses or
/// replaces any uncommitted artefact for that sequence. Members are removed
/// as by `pack revoke`: runtime authority is denied and their Fangs closed
/// before the commit, and the denial stays if the commit fails.
async fn apply_statement(
    statement: &werewolf_core::governance::Statement,
    expect: Option<&werewolf_core::governance::Hash>,
    state: &Arc<Mutex<DaemonState>>,
) -> Result<serde_json::Value, (&'static str, String)> {
    use werewolf_core::governance::{self, tool, GovError, Outcome};
    let (head, generation, peers) = {
        let live = state.lock().await;
        (
            live.governance.clone(),
            live.protected_state_generation,
            live.peers.clone(),
        )
    };
    let Some(generation) = generation else {
        return Err((
            "GOVERNANCE_NEEDS_MANIFEST",
            "governance requires the protected manifest; run state manifest-migrate".into(),
        ));
    };
    let next = match governance::apply(head.as_ref(), statement, expect) {
        Ok(Outcome::AlreadyApplied) => {
            return Ok(json!({"status": "already_applied", "sequence": statement.core.sequence}))
        }
        Ok(Outcome::Applied(next)) => next,
        Ok(Outcome::Fork(fenced)) => {
            // Record the fork durably, then refuse: governance is fenced
            // until an operator intervenes (spec S3).
            let mut protected = protected_from_live(&*state.lock().await);
            protected.governance = Some(fenced.clone());
            if commit_protected(generation, protected, state).await.is_ok() {
                state.lock().await.governance = Some(fenced);
            }
            return Err((
                "GOVERNANCE_FORK",
                format!(
                    "a validly signed statement conflicts with applied sequence {}; governance is fenced",
                    statement.core.sequence
                ),
            ));
        }
        Err(GovError::Forked) => return Err(("GOVERNANCE_FORKED", GovError::Forked.to_string())),
        Err(error) => return Err(("GOVERNANCE_REJECTED", error.to_string())),
    };

    // G3: archive before anything changes; a failure leaves no effect.
    let den = state.lock().await.den.clone();
    let archived = match den {
        Some(den) => matches!(
            den.replace(
                std::ffi::OsStr::new(&crate::discovery::governance_archive_name(next.sequence)),
                &statement.encode(),
                false,
            ),
            CommitOutcome::DurablyCommitted
        ),
        None => false,
    };
    if !archived {
        return Err((
            "GOVERNANCE_SAVE_FAILED",
            "the statement could not be archived".into(),
        ));
    }

    // Pack members the new head excludes: a revoked key, or after activation
    // a key not on the roster (Stage29 sweep). The Pack held none before,
    // so these are exactly the members this statement removes.
    let revoked: Vec<PeerRecord> = peers
        .iter()
        .filter(|peer| crate::governance::excludes(&next, peer))
        .cloned()
        .collect();
    // Stage29 local admission before and after this statement.
    let (was_admitted, now_admitted) = {
        let live = state.lock().await;
        let own = crate::governance::own_key(&live);
        let admits = |h: &governance::Head| match own {
            Some(key) => h.admits(&key),
            None => h.activated_at.is_none(),
        };
        (head.as_ref().is_none_or(admits), admits(&next))
    };
    let candidate: Vec<PeerRecord> = peers
        .iter()
        .filter(|peer| !revoked.iter().any(|r| r.name == peer.name))
        .cloned()
        .collect();

    let (authority, closed, reopen_epoch) = {
        let mut live = state.lock().await;
        let authority = live.inbound_authority.clone();
        let mut closed = 0;
        // Becoming unadmitted fences the node first, before anything is
        // saved: no inbound session or RPC, and every Fang closed.
        if was_admitted && !now_admitted {
            if authority.lock().is_err() {
                return Err((
                    "AUTHORITY_UNAVAILABLE",
                    "runtime authority unavailable".into(),
                ));
            }
            closed += live.fang_registry.len();
            live.fang_registry.abort_all();
            live.fang_registry.clear_started();
            live.fang_registry.clear_records();
        }
        for peer in &revoked {
            if authority.deny_peer(&peer.fingerprint).is_err() {
                return Err((
                    "AUTHORITY_UNAVAILABLE",
                    "runtime authority unavailable".into(),
                ));
            }
            closed += live.fang_registry.terminate_peer(&peer.name);
        }
        live.status.active_fangs = live.fang_registry.len();
        // The epoch a later reopening must still match (as Silver reset).
        let reopen_epoch = authority.epoch().ok();
        (authority, closed, reopen_epoch)
    };
    let mut protected = protected_from_live(&*state.lock().await);
    protected.peers = candidate.clone();
    protected.governance = Some(next.clone());
    if commit_protected(generation, protected, state)
        .await
        .is_err()
    {
        // Runtime denial is never restored on a failed save; the archive
        // entry stays an uncommitted artefact, never served (G3).
        return Err((
            if revoked.is_empty() {
                "GOVERNANCE_SAVE_FAILED"
            } else {
                "GOVERNANCE_SAVE_FAILED_RUNTIME_DENIED"
            },
            "the governance head was not saved".into(),
        ));
    }
    {
        let mut live = state.lock().await;
        live.peers = candidate;
        live.governance = Some(next.clone());
        for peer in &revoked {
            if authority.removed(&peer.fingerprint).is_err() {
                live.storage_degraded = true;
                return Err((
                    "AUTHORITY_PUBLICATION_FAILED",
                    "saved, but runtime authority could not be published".into(),
                ));
            }
        }
    }
    for peer in &revoked {
        if authority.cleanup(Some(&peer.fingerprint)).await.is_err() {
            return Err((
                "AUTHORITY_CLEANUP_TIMEOUT",
                "saved; authority cleanup timed out".into(),
            ));
        }
    }
    // Stage30: the DEAD head is durably committed and published, the point
    // of no return. Destruction follows; the process then exits 67.
    if next.dead_at.is_some() {
        return Ok(crate::terminal::die(state, statement.clone()).await);
    }
    // Admitted again (or for the first time after activation): the fence
    // lifts, but only if Silver is not holding the authority locked too.
    if !was_admitted && now_admitted {
        let silver = state.lock().await.status.silver == "active";
        if !silver {
            if let Some(epoch) = reopen_epoch {
                let _ = authority.open_after_durable(epoch);
            }
        }
    }
    Ok(json!({
        "status": "applied",
        "sequence": next.sequence,
        "hash": tool::hex(&next.hash),
        "revoked_members": revoked.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
        "closed_fangs": closed,
        "admitted": now_admitted,
    }))
}

fn publish_invalidated_runtime(
    live: &mut DaemonState,
    candidate_active: &[String],
    affected_ids: &[String],
) -> FangTermination {
    live.active_profiles = candidate_active.to_vec();
    let termination = live.fang_registry.detach_ids(affected_ids);
    if termination.closed() != 0 {
        live.status.active_fangs = live.fang_registry.len();
        if live.fang_registry.is_empty() {
            live.status.mode = werewolf_core::state::WolfMode::Human;
        }
    }
    termination
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

    fn test_service(name: &str) -> ServiceDefinition {
        ServiceDefinition {
            name: name.into(),
            peer: "wolf-not-yet-in-pack".into(),
            target: "127.0.0.1:8080".into(),
            kind: ServiceKind::Http,
            fallbacks: Vec::new(),
        }
    }

    fn service_add_request(name: &str) -> ControlRequest {
        ControlRequest {
            id: "service-add".into(),
            cmd: "service.add".into(),
            args: json!({
                "name": name,
                "peer": "wolf-not-yet-in-pack",
                "target": "127.0.0.1:8080",
                "kind": "http"
            }),
        }
    }

    #[tokio::test]
    async fn service_add_persists_and_publishes_without_requiring_pack_peer() {
        let fixture = Fixture::new();
        let state = fixture.state();

        assert!(state.lock().await.peers.is_empty());

        let response = handle(service_add_request("web"), state.clone(), fixture.0.clone()).await;

        assert!(response.ok);

        let expected = vec![test_service("web")];

        assert_eq!(state.lock().await.services, expected);

        let disk: Vec<ServiceDefinition> =
            serde_json::from_slice(&std::fs::read(fixture.0.join("services.json")).unwrap())
                .unwrap();

        assert_eq!(disk, expected);
    }

    #[tokio::test]
    async fn service_invalid_and_duplicate_adds_do_not_rewrite_registry() {
        let fixture = Fixture::new();
        let state = fixture.state();

        let invalid_kind = handle(
            ControlRequest {
                id: "invalid-kind".into(),
                cmd: "service.add".into(),
                args: json!({
                    "name": "web",
                    "peer": "wolf-a",
                    "target": "127.0.0.1:8080",
                    "kind": "smtp"
                }),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;

        assert!(!invalid_kind.ok);
        assert_eq!(invalid_kind.error.unwrap().code, "SERVICE_INVALID");
        assert!(state.lock().await.services.is_empty());
        assert!(!fixture.0.join("services.json").exists());

        let invalid_peer = handle(
            ControlRequest {
                id: "invalid-peer".into(),
                cmd: "service.add".into(),
                args: json!({
                    "name": "web",
                    "peer": "",
                    "target": "127.0.0.1:8080",
                    "kind": "http"
                }),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;

        assert!(!invalid_peer.ok);
        assert_eq!(invalid_peer.error.unwrap().code, "SERVICE_INVALID");
        assert!(state.lock().await.services.is_empty());
        assert!(!fixture.0.join("services.json").exists());

        assert!(
            handle(service_add_request("web"), state.clone(), fixture.0.clone(),)
                .await
                .ok
        );

        let before = std::fs::read(fixture.0.join("services.json")).unwrap();

        let duplicate = handle(service_add_request("web"), state.clone(), fixture.0.clone()).await;

        assert!(!duplicate.ok);
        assert_eq!(duplicate.error.unwrap().code, "SERVICE_EXISTS");
        assert_eq!(state.lock().await.services, vec![test_service("web")]);
        assert_eq!(
            std::fs::read(fixture.0.join("services.json")).unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn service_remove_persists_and_publishes_and_missing_is_idempotent() {
        let fixture = Fixture::new();
        let state = fixture.state();

        assert!(
            handle(service_add_request("web"), state.clone(), fixture.0.clone(),)
                .await
                .ok
        );

        let remove = || ControlRequest {
            id: "service-remove".into(),
            cmd: "service.remove".into(),
            args: json!({"name": "web"}),
        };

        let response = handle(remove(), state.clone(), fixture.0.clone()).await;
        assert!(response.ok);
        assert!(state.lock().await.services.is_empty());

        let disk: Vec<ServiceDefinition> =
            serde_json::from_slice(&std::fs::read(fixture.0.join("services.json")).unwrap())
                .unwrap();

        assert!(disk.is_empty());

        let committed = std::fs::read(fixture.0.join("services.json")).unwrap();

        let missing = handle(remove(), state.clone(), fixture.0.clone()).await;

        assert!(!missing.ok);
        assert_eq!(missing.error.unwrap().code, "SERVICE_NOT_FOUND");
        assert!(state.lock().await.services.is_empty());
        assert_eq!(
            std::fs::read(fixture.0.join("services.json")).unwrap(),
            committed
        );
    }

    #[tokio::test]
    async fn failed_service_add_save_preserves_live_memory() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("services.json")).unwrap();

        let state = fixture.state();

        let response = handle(service_add_request("web"), state.clone(), fixture.0.clone()).await;

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "SERVICE_SAVE_FAILED");
        assert!(state.lock().await.services.is_empty());
        assert!(!state.lock().await.storage_degraded);
        assert!(fixture.0.join("services.json").is_dir());
    }

    #[tokio::test]
    async fn failed_service_remove_save_preserves_live_memory() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("services.json")).unwrap();

        let state = Arc::new(Mutex::new(DaemonState {
            services: vec![test_service("web")],
            den: Some(Arc::new(PrivateDirectory::open(&fixture.0, false).unwrap())),
            ..DaemonState::default()
        }));

        let response = handle(
            ControlRequest {
                id: "service-remove".into(),
                cmd: "service.remove".into(),
                args: json!({"name": "web"}),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "SERVICE_SAVE_FAILED");
        assert_eq!(state.lock().await.services, vec![test_service("web")]);
        assert!(!state.lock().await.storage_degraded);
        assert!(fixture.0.join("services.json").is_dir());
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
    async fn incremental_target_mutations_use_the_protected_generation() {
        let fixture = Fixture::new();
        let state = fixture.state();
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
        let peer = generate_identity();
        assert!(handle(
            ControlRequest { id: "pack".into(), cmd: "pack.add".into(), args: json!({
                "name":"peer", "fingerprint":peer.fingerprint, "address":"tcp://127.0.0.1:1",
                "public_key_b64":peer.public_key_b64
            }) }, state.clone(), fixture.0.clone()
        ).await.ok);
        assert!(
            handle(
                ControlRequest {
                    id: "allow".into(),
                    cmd: "target.allow".into(),
                    args: json!({"peer":"peer","target":"127.0.0.1:1"})
                },
                state.clone(),
                fixture.0.clone()
            )
            .await
            .ok
        );
        assert_eq!(state.lock().await.protected_state_generation, Some(3));
        assert!(
            handle(
                ControlRequest {
                    id: "remove".into(),
                    cmd: "target.remove".into(),
                    args: json!({"peer":"peer","target":"127.0.0.1:1"})
                },
                state.clone(),
                fixture.0.clone()
            )
            .await
            .ok
        );
        assert_eq!(state.lock().await.protected_state_generation, Some(4));
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

    // Stage25 `service.publish` / `service.unpublish`.

    fn publish(name: &str, target: &str, kind: &str) -> ControlRequest {
        ControlRequest {
            id: "publish".into(),
            cmd: "service.publish".into(),
            args: json!({"name": name, "target": target, "kind": kind}),
        }
    }

    fn unpublish(name: &str) -> ControlRequest {
        ControlRequest {
            id: "unpublish".into(),
            cmd: "service.unpublish".into(),
            args: json!({"name": name}),
        }
    }

    fn published_on_disk(fixture: &Fixture) -> Vec<werewolf_core::published::PublishedService> {
        crate::discovery::operator::parse_published(
            &std::fs::read(fixture.0.join("published.json")).unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn publish_persists_then_publishes_and_unpublish_removes() {
        let fixture = Fixture::new();
        let state = fixture.state();

        let response = handle(
            publish("web", "127.0.0.1:8080", "http"),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(response.ok, "{response:?}");
        let live = state.lock().await.published.clone();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].target, "127.0.0.1:8080");
        assert_eq!(published_on_disk(&fixture), live);
        // Publishing grants nothing: no Pack peer or target grant is needed
        // or created.
        assert!(matches!(
            state.lock().await.target_policy,
            crate::target_policy::TargetPolicy::Deny
        ));

        let response = handle(unpublish("web"), state.clone(), fixture.0.clone()).await;
        assert!(response.ok, "{response:?}");
        assert!(state.lock().await.published.is_empty());
        assert!(published_on_disk(&fixture).is_empty());
    }

    #[tokio::test]
    async fn invalid_duplicate_and_missing_publications_change_nothing() {
        let fixture = Fixture::new();
        let state = fixture.state();
        assert!(
            handle(
                publish("web", "127.0.0.1:8080", "http"),
                state.clone(),
                fixture.0.clone()
            )
            .await
            .ok
        );
        let committed = std::fs::read(fixture.0.join("published.json")).unwrap();

        for (request, code) in [
            (publish("web", "127.0.0.1:9090", "tcp"), "SERVICE_EXISTS"),
            (publish("db", "db.internal:5432", "tcp"), "SERVICE_INVALID"),
            (publish("db", "0.0.0.0:5432", "tcp"), "SERVICE_INVALID"),
            (publish("db", "127.0.0.1:5432", "ftp"), "SERVICE_INVALID"),
            (
                publish("d\u{1b}b", "127.0.0.1:5432", "tcp"),
                "SERVICE_INVALID",
            ),
            (publish("", "127.0.0.1:5432", "tcp"), "SERVICE_INVALID"),
            (unpublish("db"), "SERVICE_NOT_FOUND"),
        ] {
            let response = handle(request, state.clone(), fixture.0.clone()).await;
            assert!(!response.ok);
            assert_eq!(response.error.unwrap().code, code);
        }
        assert_eq!(state.lock().await.published.len(), 1);
        assert_eq!(
            std::fs::read(fixture.0.join("published.json")).unwrap(),
            committed
        );
    }

    #[tokio::test]
    async fn publication_count_is_capped() {
        let fixture = Fixture::new();
        let state = fixture.state();
        for i in 0..werewolf_core::published::MAX_PUBLISHED {
            let name = format!("s{i}");
            assert!(
                handle(
                    publish(&name, "127.0.0.1:8080", "tcp"),
                    state.clone(),
                    fixture.0.clone()
                )
                .await
                .ok
            );
        }
        let response = handle(
            publish("one-more", "127.0.0.1:8080", "tcp"),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert_eq!(response.error.unwrap().code, "SERVICE_INVALID");
        assert_eq!(
            state.lock().await.published.len(),
            werewolf_core::published::MAX_PUBLISHED
        );
    }

    #[tokio::test]
    async fn failed_publish_save_preserves_live_memory() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("published.json")).unwrap();
        let state = fixture.state();

        let response = handle(
            publish("web", "127.0.0.1:8080", "http"),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert_eq!(response.error.unwrap().code, "SERVICE_SAVE_FAILED");
        assert!(state.lock().await.published.is_empty());
        assert!(!state.lock().await.storage_degraded);
    }

    // Stage26 `route.set` / `route.remove`.

    fn set_route(peer: &str, via: &str, hop: Option<&str>) -> ControlRequest {
        let mut args = json!({"peer": peer, "via": via});
        if let Some(hop) = hop {
            args["hop"] = hop.into();
        }
        ControlRequest {
            id: "route".into(),
            cmd: "route.set".into(),
            args,
        }
    }

    fn remove_route(peer: &str) -> ControlRequest {
        ControlRequest {
            id: "route".into(),
            cmd: "route.remove".into(),
            args: json!({"peer": peer}),
        }
    }

    fn routes_on_disk(fixture: &Fixture) -> Vec<werewolf_core::route::Route> {
        crate::persistence::parse_routes(&std::fs::read(fixture.0.join("routes.json")).unwrap())
            .unwrap()
    }

    async fn with_members(state: &Arc<Mutex<DaemonState>>, names: &[&str]) {
        let mut st = state.lock().await;
        for name in names {
            st.peers.push(PeerRecord {
                name: (*name).into(),
                fingerprint: format!("wwp1:{name}"),
                address: "tcp://127.0.0.1:1".into(),
                trust: TrustLevel::Packmate,
                public_key_b64: None,
            });
        }
    }

    #[tokio::test]
    async fn route_set_persists_then_publishes_updates_and_removes() {
        let fixture = Fixture::new();
        let state = fixture.state();
        with_members(&state, &["wolf-b", "wolf-c", "wolf-d"]).await;

        let response = handle(
            set_route("wolf-d", "wolf-b", None),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(response.ok, "{response:?}");
        assert_eq!(response.result.unwrap()["status"], "set");
        let live = state.lock().await.routes.clone();
        assert_eq!(
            live,
            [werewolf_core::route::Route {
                peer: "wolf-d".into(),
                via: "wolf-b".into(),
                hop: werewolf_core::route::RouteHop::Quic,
            }]
        );
        assert_eq!(routes_on_disk(&fixture), live);

        // Setting again replaces the one route for that peer.
        let response = handle(
            set_route("wolf-d", "wolf-c", Some("tcp")),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert_eq!(response.result.unwrap()["status"], "updated");
        let live = state.lock().await.routes.clone();
        assert_eq!(live.len(), 1);
        assert_eq!(
            (live[0].via.as_str(), live[0].hop.as_str()),
            ("wolf-c", "tcp")
        );
        assert_eq!(routes_on_disk(&fixture), live);
        // A route grants nothing and is outside the protected manifest.
        assert!(matches!(
            state.lock().await.target_policy,
            crate::target_policy::TargetPolicy::Deny
        ));
        assert_eq!(state.lock().await.protected_state_generation, None);

        let response = handle(remove_route("wolf-d"), state.clone(), fixture.0.clone()).await;
        assert!(response.ok, "{response:?}");
        assert!(state.lock().await.routes.is_empty());
        assert!(routes_on_disk(&fixture).is_empty());
    }

    #[tokio::test]
    async fn invalid_routes_change_nothing() {
        let fixture = Fixture::new();
        let state = fixture.state();
        with_members(&state, &["wolf-b", "wolf-c", "wolf-d"]).await;
        assert!(
            handle(
                set_route("wolf-d", "wolf-b", None),
                state.clone(),
                fixture.0.clone()
            )
            .await
            .ok
        );
        let committed = std::fs::read(fixture.0.join("routes.json")).unwrap();

        for (request, code) in [
            (set_route("wolf-x", "wolf-b", None), "ROUTE_INVALID"), // not in the Pack
            (set_route("wolf-c", "wolf-x", None), "ROUTE_INVALID"),
            (set_route("wolf-c", "wolf-c", None), "ROUTE_INVALID"),
            (
                set_route("wolf-c", "wolf-b", Some("tcp-plain")),
                "ROUTE_INVALID",
            ),
            (set_route("wolf-c", "wolf-b", Some("udp")), "ROUTE_INVALID"),
            (set_route("", "wolf-b", None), "ROUTE_INVALID"),
            // wolf-b relays for wolf-d, so it cannot be routed itself, and a
            // routed peer cannot relay (exactly one relay).
            (set_route("wolf-b", "wolf-c", None), "ROUTE_INVALID"),
            (set_route("wolf-c", "wolf-d", None), "ROUTE_INVALID"),
            (remove_route("wolf-c"), "ROUTE_NOT_FOUND"),
        ] {
            let response = handle(request, state.clone(), fixture.0.clone()).await;
            assert!(!response.ok);
            assert_eq!(response.error.unwrap().code, code);
        }
        assert_eq!(state.lock().await.routes.len(), 1);
        assert_eq!(
            std::fs::read(fixture.0.join("routes.json")).unwrap(),
            committed
        );
    }

    #[tokio::test]
    async fn failed_route_save_preserves_live_memory() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("routes.json")).unwrap();
        let state = fixture.state();
        with_members(&state, &["wolf-b", "wolf-d"]).await;

        let response = handle(
            set_route("wolf-d", "wolf-b", None),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert_eq!(response.error.unwrap().code, "ROUTE_SAVE_FAILED");
        assert!(state.lock().await.routes.is_empty());
        assert!(!state.lock().await.storage_degraded);
    }

    // Stage27 `service.provider.add` / `service.provider.remove`.

    fn provider_request(cmd: &str, args: serde_json::Value) -> ControlRequest {
        ControlRequest {
            id: "provider".into(),
            cmd: cmd.into(),
            args,
        }
    }

    fn services_on_disk(fixture: &Fixture) -> Vec<ServiceDefinition> {
        serde_json::from_slice(&std::fs::read(fixture.0.join("services.json")).unwrap()).unwrap()
    }

    fn order(state: &DaemonState) -> Vec<String> {
        state.services[0]
            .providers()
            .map(|(peer, _)| peer.to_owned())
            .collect()
    }

    #[tokio::test]
    async fn providers_are_added_in_order_and_removed_with_promotion() {
        let fixture = Fixture::new();
        let state = fixture.state();
        assert!(
            handle(service_add_request("web"), state.clone(), fixture.0.clone())
                .await
                .ok
        );
        for peer in ["wolf-b", "wolf-c"] {
            let response = handle(
                provider_request(
                    "service.provider.add",
                    json!({"name": "web", "peer": peer, "target": "127.0.0.1:8081"}),
                ),
                state.clone(),
                fixture.0.clone(),
            )
            .await;
            assert!(response.ok, "{response:?}");
        }
        assert_eq!(
            order(&*state.lock().await),
            ["wolf-not-yet-in-pack", "wolf-b", "wolf-c"]
        );
        assert_eq!(services_on_disk(&fixture), state.lock().await.services);

        // Removing the first provider promotes the next, target included.
        let response = handle(
            provider_request(
                "service.provider.remove",
                json!({"name": "web", "peer": "wolf-not-yet-in-pack"}),
            ),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert_eq!(response.result.unwrap()["providers"], 2);
        {
            let live = state.lock().await;
            assert_eq!(order(&live), ["wolf-b", "wolf-c"]);
            assert_eq!(live.services[0].target, "127.0.0.1:8081");
        }
        let response = handle(
            provider_request(
                "service.provider.remove",
                json!({"name": "web", "peer": "wolf-c"}),
            ),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert!(response.ok);
        assert_eq!(order(&*state.lock().await), ["wolf-b"]);
        // Back to one provider: the file is in its Stage23 form again.
        assert!(!std::fs::read_to_string(fixture.0.join("services.json"))
            .unwrap()
            .contains("fallbacks"));
    }

    #[tokio::test]
    async fn invalid_provider_changes_leave_the_registry_unchanged() {
        let fixture = Fixture::new();
        let state = fixture.state();
        assert!(
            handle(service_add_request("web"), state.clone(), fixture.0.clone())
                .await
                .ok
        );
        let add = |name: &str, peer: &str, kind: Option<&str>| {
            let mut args = json!({"name": name, "peer": peer, "target": "127.0.0.1:8081"});
            if let Some(kind) = kind {
                args["kind"] = kind.into();
            }
            provider_request("service.provider.add", args)
        };
        for peer in ["wolf-b", "wolf-c", "wolf-d"] {
            assert!(
                handle(add("web", peer, None), state.clone(), fixture.0.clone())
                    .await
                    .ok
            );
        }
        let committed = std::fs::read(fixture.0.join("services.json")).unwrap();
        for (request, code) in [
            (add("web", "wolf-e", None), "SERVICE_INVALID"), // a fifth provider
            (add("web", "wolf-b", None), "SERVICE_INVALID"), // listed twice
            (add("web", "wolf-not-yet-in-pack", None), "SERVICE_INVALID"),
            (add("ghost", "wolf-e", None), "SERVICE_NOT_FOUND"),
            (
                provider_request(
                    "service.provider.remove",
                    json!({"name": "web", "peer": "wolf-x"}),
                ),
                "SERVICE_NOT_FOUND",
            ),
        ] {
            let response = handle(request, state.clone(), fixture.0.clone()).await;
            assert_eq!(response.error.unwrap().code, code);
        }
        assert_eq!(
            std::fs::read(fixture.0.join("services.json")).unwrap(),
            committed
        );

        // From --from, a different kind under the same name is refused.
        let fixture = Fixture::new();
        let state = fixture.state();
        assert!(
            handle(service_add_request("web"), state.clone(), fixture.0.clone())
                .await
                .ok
        );
        let response = handle(
            add("web", "wolf-b", Some("ssh")),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert_eq!(response.error.unwrap().code, "SERVICE_INVALID");
        assert!(
            handle(
                add("web", "wolf-b", Some("http")),
                state.clone(),
                fixture.0.clone()
            )
            .await
            .ok
        );

        // The last provider goes only with `service remove`.
        let fixture = Fixture::new();
        let state = fixture.state();
        assert!(
            handle(service_add_request("web"), state.clone(), fixture.0.clone())
                .await
                .ok
        );
        let response = handle(
            provider_request(
                "service.provider.remove",
                json!({"name": "web", "peer": "wolf-not-yet-in-pack"}),
            ),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert_eq!(response.error.unwrap().code, "SERVICE_INVALID");
        assert_eq!(state.lock().await.services.len(), 1);
    }

    /// Stage29 G3: the archive is part of applying. A statement that cannot
    /// be archived fails with no effect: head, Pack and manifest generation
    /// unchanged.
    #[tokio::test]
    async fn a_statement_that_cannot_be_archived_has_no_effect() {
        use ed25519_dalek::SigningKey;
        use werewolf_core::governance::{sign, Body, Core, Statement};
        let fixture = Fixture::new();
        let state = fixture.state();
        let den = state.lock().await.den.clone().unwrap();
        let protected_state::Transaction::DurablyCommitted(generation) =
            protected_state::migrate(&den)
        else {
            panic!("migration failed");
        };
        state.lock().await.protected_state_generation = Some(generation.number);
        let key = |i: u8| SigningKey::from_bytes(&[i + 1; 32]);
        let signed = |core: Core| Statement {
            signatures: (0..7u8).map(|i| (i, sign(&core, &key(i)))).collect(),
            core,
        };
        let genesis = signed(Core {
            network_id: [0x77; 32],
            sequence: 0,
            previous: [0; 32],
            body: Body::Genesis {
                threshold: 7,
                keys: (0..10).map(|i| key(i).verifying_key().to_bytes()).collect(),
            },
        });
        apply_statement(&genesis, Some(&genesis.core.hash()), &state)
            .await
            .unwrap();
        let before = state.lock().await.protected_state_generation;
        let member = SigningKey::from_bytes(&[0xD0; 32])
            .verifying_key()
            .to_bytes();
        let admission = signed(Core {
            network_id: [0x77; 32],
            sequence: 1,
            previous: genesis.core.hash(),
            body: Body::Admission(vec![member]),
        });
        // A directory where the archive file must go: the write fails.
        std::fs::create_dir(fixture.0.join("governance_00000001.wwgov")).unwrap();
        let error = apply_statement(&admission, None, &state).await.unwrap_err();
        assert_eq!(error.0, "GOVERNANCE_SAVE_FAILED");
        let live = state.lock().await;
        assert_eq!(live.governance.as_ref().unwrap().sequence, 0);
        assert!(live.governance.as_ref().unwrap().roster.is_empty());
        assert_eq!(live.protected_state_generation, before);
        drop(live);
        // Once the archive can be written, the same statement applies, and
        // its archive entry is the statement.
        std::fs::remove_dir(fixture.0.join("governance_00000001.wwgov")).unwrap();
        apply_statement(&admission, None, &state).await.unwrap();
        assert_eq!(state.lock().await.governance.as_ref().unwrap().sequence, 1);
        assert_eq!(
            std::fs::read(fixture.0.join("governance_00000001.wwgov")).unwrap(),
            admission.encode()
        );
    }

    /// Stage30: genesis then a death statement on a migrated Den, with a
    /// Pelt. Returns the state after the death reply.
    async fn dead_node(fixture: &Fixture) -> (Arc<Mutex<DaemonState>>, serde_json::Value) {
        use ed25519_dalek::SigningKey;
        use werewolf_core::governance::{sign, Body, Core, Statement};
        let state = fixture.state();
        fixture.write(
            "pelt.json",
            &serde_json::to_vec(&generate_identity()).unwrap(),
        );
        let den = state.lock().await.den.clone().unwrap();
        let protected_state::Transaction::DurablyCommitted(generation) =
            protected_state::migrate(&den)
        else {
            panic!("migration failed");
        };
        state.lock().await.protected_state_generation = Some(generation.number);
        let key = |i: u8| SigningKey::from_bytes(&[i + 1; 32]);
        let signed = |core: Core| Statement {
            signatures: (0..7u8).map(|i| (i, sign(&core, &key(i)))).collect(),
            core,
        };
        let genesis = signed(Core {
            network_id: [0x77; 32],
            sequence: 0,
            previous: [0; 32],
            body: Body::Genesis {
                threshold: 7,
                keys: (0..10).map(|i| key(i).verifying_key().to_bytes()).collect(),
            },
        });
        apply_statement(&genesis, Some(&genesis.core.hash()), &state)
            .await
            .unwrap();
        let death = signed(Core {
            network_id: [0x77; 32],
            sequence: 1,
            previous: genesis.core.hash(),
            body: Body::Death,
        });
        let reply = apply_statement(&death, None, &state).await.unwrap();
        (state, reply)
    }

    fn den_names(fixture: &Fixture) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&fixture.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Stage30: a death statement commits DEAD, locks the node, and leaves
    /// only the Tombstone (no `.den.lock` here: the test holds none).
    #[tokio::test]
    async fn a_death_statement_destroys_the_node_and_leaves_a_tombstone() {
        let fixture = Fixture::new();
        let (state, reply) = dead_node(&fixture).await;
        assert_eq!(reply["status"], "applied");
        assert_eq!(reply["lifecycle"], "dead");
        assert_eq!(reply["destruction"], "complete");
        assert_eq!(den_names(&fixture), vec!["tombstone.json"]);
        let tombstone = werewolf_core::tombstone::Tombstone::parse(
            &std::fs::read(fixture.0.join("tombstone.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(tombstone.kind(), werewolf_core::tombstone::Kind::Death);
        let live = state.lock().await;
        assert!(live.dying && live.terminal_exit_requested);
        assert!(live.inbound_authority.is_locked());
        assert_eq!(live.governance.as_ref().unwrap().dead_at, Some(1));
    }

    #[tokio::test]
    async fn retirement_needs_the_exact_fingerprint_and_a_durable_tombstone() {
        let fixture = Fixture::new();
        let state = fixture.state_with_silver_open();
        let identity = generate_identity();
        fixture.write("pelt.json", &serde_json::to_vec(&identity).unwrap());
        fixture.write("routes.json", b"[]");
        state.lock().await.pelt = Some(identity.clone());

        // The wrong fingerprint changes nothing.
        let error = crate::terminal::retire(&state, "wwp1:00")
            .await
            .unwrap_err();
        assert_eq!(error.0, "RETIRE_CONFIRM_MISMATCH");
        assert!(!state.lock().await.dying);

        // A Tombstone that cannot be written: nothing happens, and the node
        // unlocks again.
        std::fs::create_dir(fixture.0.join("tombstone.json")).unwrap();
        let error = crate::terminal::retire(&state, &identity.fingerprint)
            .await
            .unwrap_err();
        assert_eq!(error.0, "RETIRE_SAVE_FAILED");
        {
            let live = state.lock().await;
            assert!(!live.dying);
            assert!(!live.inbound_authority.is_locked());
        }
        assert!(den_names(&fixture).contains(&"pelt.json".to_string()));
        std::fs::remove_dir(fixture.0.join("tombstone.json")).unwrap();

        // With storage working: retired, only the Tombstone remains.
        let result = crate::terminal::retire(&state, &identity.fingerprint)
            .await
            .unwrap();
        assert_eq!(
            result,
            serde_json::json!({"status": "retired", "destruction": "complete"})
        );
        assert_eq!(den_names(&fixture), vec!["tombstone.json"]);
        let live = state.lock().await;
        assert!(live.dying && live.terminal_exit_requested);
        assert!(live.inbound_authority.is_locked());
    }

    #[tokio::test]
    async fn a_dying_node_answers_only_status() {
        let fixture = Fixture::new();
        let state = fixture.state();
        state.lock().await.dying = true;
        let ask = |cmd: &str| ControlRequest {
            id: "1".into(),
            cmd: cmd.into(),
            args: serde_json::json!({}),
        };
        for cmd in [
            "pack.list",
            "pelt.init",
            "governance.apply",
            "silver.reset",
            "den.retire",
        ] {
            let response =
                crate::control::handle_request(ask(cmd), state.clone(), fixture.0.clone()).await;
            assert_eq!(response.error.unwrap().code, "LIFECYCLE_DYING", "{cmd}");
        }
        let status =
            crate::control::handle_request(ask("status"), state.clone(), fixture.0.clone()).await;
        assert_eq!(status.result.unwrap()["lifecycle"], "DYING");
        let governance = crate::control::handle_request(
            ask("governance.status"),
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert_eq!(
            governance.result.unwrap()["lifecycle"],
            serde_json::json!({"chain": "none", "node": "dying"})
        );
    }

    #[tokio::test]
    async fn a_den_with_a_tombstone_is_never_initialized_again() {
        let fixture = Fixture::new();
        let state = fixture.state();
        fixture.write(
            "tombstone.json",
            &werewolf_core::tombstone::Tombstone::retirement(
                werewolf_core::tombstone::Destruction::Complete,
                "werewolfd test",
            )
            .unwrap()
            .encode()
            .unwrap(),
        );
        let response = handle(
            ControlRequest {
                id: "1".into(),
                cmd: "pelt.init".into(),
                args: serde_json::json!({}),
            },
            state.clone(),
            fixture.0.clone(),
        )
        .await;
        assert_eq!(response.error.as_ref().unwrap().code, "LIFECYCLE_TOMBSTONE");
        assert!(state.lock().await.pelt.is_none());
    }
}
