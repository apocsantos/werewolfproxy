use crate::{
    authority::Authority,
    protected_state::{self, Load, ProtectedState},
    state::DaemonState,
    target_policy,
};
use std::{ffi::OsStr, io};
use werewolf_core::{local_fs::PrivateDirectory, state_validation};
use zeroize::Zeroizing;

pub(super) fn load_startup_state(
    den: &PrivateDirectory,
    initial_state: &mut DaemonState,
) -> io::Result<()> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Silver {
        version: u8,
        mode: String,
    }
    match protected_state::load(den)? {
        Load::Manifest { generation, state } => {
            apply_protected_state(initial_state, state, Some(generation.number));
        }
        Load::Legacy => {
            let silver_locked = match den.read(OsStr::new("silver.json"), 1024)? {
                None => true,
                Some(data) => {
                    let value: Silver = serde_json::from_slice(&data).map_err(|_| invalid())?;
                    if value.version != 1 {
                        return Err(invalid());
                    }
                    match value.mode.as_str() {
                        "locked" => true,
                        "open" => false,
                        _ => return Err(invalid()),
                    }
                }
            };
            let state = ProtectedState {
                silver_locked,
                target_policy: match den.read(OsStr::new("target_policy.json"), 1024 * 1024)? {
                    None => target_policy::TargetPolicy::Deny,
                    Some(data) => match std::str::from_utf8(&data) {
                        Ok(text) => target_policy::parse(text),
                        Err(_) => target_policy::TargetPolicy::Invalid,
                    },
                },
                peers: match den.read(OsStr::new("pack.json"), 1024 * 1024)? {
                    None => Vec::new(),
                    Some(data) => {
                        let peers =
                            serde_json::from_slice::<Vec<_>>(&data).map_err(|_| invalid())?;
                        state_validation::pack(&peers)?;
                        peers
                    }
                },
                fang_profiles: match den.read(OsStr::new("fangs.json"), 1024 * 1024)? {
                    None => Vec::new(),
                    Some(data) => {
                        let profiles =
                            serde_json::from_slice::<Vec<_>>(&data).map_err(|_| invalid())?;
                        state_validation::profile_document(&profiles)?;
                        profiles
                    }
                },
                active_profiles: Vec::new(),
            };
            let mut state = state;
            state.active_profiles = match den.read(OsStr::new("active_fangs.json"), 1024 * 1024)? {
                None => Vec::new(),
                Some(data) => {
                    let active =
                        serde_json::from_slice::<Vec<String>>(&data).map_err(|_| invalid())?;
                    state_validation::active(&active, &state.fang_profiles)?;
                    active
                }
            };
            apply_protected_state(initial_state, state, None);
        }
    }
    if let Some(data) = den.read(OsStr::new("pelt.json"), 4096)? {
        let data = Zeroizing::new(data);
        let pelt = serde_json::from_slice(&data).map_err(|_| invalid())?;
        state_validation::identity(&pelt)?;
        initial_state.pelt = Some(pelt);
        initial_state.status.pelt_ready = true;
    }
    Ok(())
}

fn apply_protected_state(
    initial_state: &mut DaemonState,
    state: ProtectedState,
    generation: Option<u64>,
) {
    initial_state.inbound_authority = Authority::new(state.silver_locked);
    initial_state.status.silver = if state.silver_locked {
        "active".into()
    } else {
        "armed".into()
    };
    initial_state.status.mode = if state.silver_locked {
        werewolf_core::state::WolfMode::Silver
    } else {
        werewolf_core::state::WolfMode::Human
    };
    initial_state.target_policy = state.target_policy;
    initial_state.peers = state.peers;
    initial_state.fang_profiles = state.fang_profiles;
    initial_state.active_profiles = state.active_profiles;
    initial_state.protected_state_generation = generation;
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid persistent security state",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::{symlink, PermissionsExt},
        path::PathBuf,
    };
    use werewolf_core::{
        local_fs::CommitOutcome,
        pack::{PeerRecord, TrustLevel},
        pelt::generate_identity,
    };
    struct Fixture(PathBuf, PrivateDirectory);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "wwp-state-{}",
                crate::handshake::hex(&crate::handshake::random::<16>().unwrap())
            ));
            let directory = PrivateDirectory::open(&path, true).unwrap();
            assert!(matches!(
                directory.replace(
                    OsStr::new("silver.json"),
                    br#"{"version":1,"mode":"open"}"#,
                    false
                ),
                CommitOutcome::DurablyCommitted
            ));
            Self(path, directory)
        }
        fn write(&self, name: &str, data: &[u8]) {
            assert!(matches!(
                self.1.replace(OsStr::new(name), data, false),
                CommitOutcome::DurablyCommitted
            ));
        }

        fn startup(&self) -> DaemonState {
            // A new state models the next process after the previous owner
            // stopped. This invokes the production startup reader.
            let mut state = DaemonState::default();
            load_startup_state(&self.1, &mut state).unwrap();
            state
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn startup_identity_and_pack_integrity() {
        let fixture = Fixture::new();
        let identity = generate_identity();
        fixture.write("pelt.json", &serde_json::to_vec(&identity).unwrap());
        let mut state = DaemonState::default();
        load_startup_state(&fixture.1, &mut state).unwrap();
        assert!(state.status.pelt_ready);
        let mut invalid_identity = identity.clone();
        invalid_identity.fingerprint = "invalid".into();
        fixture.write("pelt.json", &serde_json::to_vec(&invalid_identity).unwrap());
        assert!(load_startup_state(&fixture.1, &mut DaemonState::default()).is_err());
        fixture.write("pelt.json", &serde_json::to_vec(&identity).unwrap());
        let peer = serde_json::json!({"name":"peer", "fingerprint":identity.fingerprint, "public_key_b64":identity.public_key_b64, "address":"quic://127.0.0.1:1", "trust":"Packmate"});
        fixture.write(
            "pack.json",
            &serde_json::to_vec(&vec![peer.clone(), peer]).unwrap(),
        );
        assert!(load_startup_state(&fixture.1, &mut DaemonState::default()).is_err());
    }

    #[tokio::test]
    async fn removed_peer_profiles_remain_inert_and_partial_temps_are_ignored() {
        let fixture = Fixture::new();
        let identity = generate_identity();
        fixture.write("pelt.json", &serde_json::to_vec(&identity).unwrap());
        fixture.write("pack.json", b"[]");
        fixture.write("fangs.json", br#"[{"name":"old","peer":"removed","local":"127.0.0.1:12345","remote":"127.0.0.1:1","transport":"tcp"}]"#);
        fixture.write("active_fangs.json", br#"["old"]"#);
        fixture.write(".interrupted.tmp", b"{partial");
        let mut state = DaemonState::default();
        load_startup_state(&fixture.1, &mut state).unwrap();
        let state = std::sync::Arc::new(tokio::sync::Mutex::new(state));
        let response = crate::open_fang_from_parts(
            "test".into(),
            state.clone(),
            "removed".into(),
            "127.0.0.1:12345".into(),
            "127.0.0.1:1".into(),
            "tcp".into(),
        )
        .await;
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "FANG_UNKNOWN_PEER");
        assert!(state.lock().await.fang_registry.is_empty());
    }

    #[test]
    fn target_policy_content_denies_but_unsafe_files_fail_startup() {
        let fixture = Fixture::new();
        let mut state = DaemonState::default();
        load_startup_state(&fixture.1, &mut state).unwrap();
        assert!(matches!(
            state.target_policy,
            target_policy::TargetPolicy::Deny
        ));
        fixture.write("target_policy.json", b"malformed");
        load_startup_state(&fixture.1, &mut state).unwrap();
        assert!(matches!(
            state.target_policy,
            target_policy::TargetPolicy::Invalid
        ));
        std::fs::set_permissions(
            fixture.0.join("target_policy.json"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(load_startup_state(&fixture.1, &mut state).is_err());
        std::fs::remove_file(fixture.0.join("target_policy.json")).unwrap();
        std::fs::write(fixture.0.join("victim"), b"untouched").unwrap();
        symlink(
            fixture.0.join("victim"),
            fixture.0.join("target_policy.json"),
        )
        .unwrap();
        assert!(load_startup_state(&fixture.1, &mut state).is_err());
        assert_eq!(
            std::fs::read(fixture.0.join("victim")).unwrap(),
            b"untouched"
        );
    }

    #[test]
    fn silver_latch_is_strict_and_missing_or_deleted_is_locked() {
        let fixture = Fixture::new();
        // Fixture starts explicitly open; startup must reflect the persisted latch.
        let mut state = DaemonState::default();
        load_startup_state(&fixture.1, &mut state).unwrap();
        assert!(!state.inbound_authority.is_locked());
        assert_eq!(state.status.silver, "armed");
        fixture.write("silver.json", br#"{"version":1,"mode":"locked"}"#);
        load_startup_state(&fixture.1, &mut state).unwrap();
        assert!(state.inbound_authority.is_locked());
        assert_eq!(state.status.silver, "active");
        fixture.write("silver.json", br#"{"version":1,"mode":"unknown"}"#);
        assert!(load_startup_state(&fixture.1, &mut DaemonState::default()).is_err());
        std::fs::remove_file(fixture.0.join("silver.json")).unwrap();
        load_startup_state(&fixture.1, &mut state).unwrap();
        assert!(state.inbound_authority.is_locked());
        assert_eq!(state.status.silver, "active");
    }

    #[test]
    fn characterizes_old_silver_open_restored_after_durable_lock() {
        let fixture = Fixture::new();
        let old_open = fixture
            .1
            .read(OsStr::new("silver.json"), 1024)
            .unwrap()
            .unwrap();
        assert!(!fixture.startup().inbound_authority.is_locked());

        fixture.write("silver.json", br#"{"version":1,"mode":"locked"}"#);
        assert!(fixture.startup().inbound_authority.is_locked());

        fixture.write("silver.json", &old_open);
        assert!(!fixture.startup().inbound_authority.is_locked());
    }

    #[test]
    fn characterizes_old_silver_lock_restored_after_newer_open() {
        let fixture = Fixture::new();
        fixture.write("silver.json", br#"{"version":1,"mode":"locked"}"#);
        let old_locked = fixture
            .1
            .read(OsStr::new("silver.json"), 1024)
            .unwrap()
            .unwrap();
        assert!(fixture.startup().inbound_authority.is_locked());

        fixture.write("silver.json", br#"{"version":1,"mode":"open"}"#);
        assert!(!fixture.startup().inbound_authority.is_locked());

        fixture.write("silver.json", &old_locked);
        assert!(fixture.startup().inbound_authority.is_locked());
    }

    #[test]
    fn characterizes_removed_pack_peer_restored_from_valid_old_document() {
        let fixture = Fixture::new();
        let peer_identity = generate_identity();
        let peer = PeerRecord {
            name: "former-peer".into(),
            fingerprint: peer_identity.fingerprint.clone(),
            address: "tcp://127.0.0.1:12345".into(),
            trust: TrustLevel::Packmate,
            public_key_b64: Some(peer_identity.public_key_b64.clone()),
        };
        let old_pack = serde_json::to_vec(&vec![peer]).unwrap();
        fixture.write("pack.json", &old_pack);
        assert_eq!(fixture.startup().peers.len(), 1);

        fixture.write("pack.json", b"[]");
        assert!(fixture.startup().peers.is_empty());

        fixture.write("pack.json", &old_pack);
        assert_eq!(fixture.startup().peers.len(), 1);
        assert_eq!(fixture.startup().peers[0].name, "former-peer");
    }

    #[tokio::test]
    async fn characterizes_old_target_grant_restored_after_removal() {
        let fixture = Fixture::new();
        let peer = generate_identity();
        let old_policy = serde_json::json!({
            "mode": "deny-by-default",
            "peers": {peer.fingerprint.clone(): {"targets": [{"address": "127.0.0.1", "port": 12345}]}}
        })
        .to_string();
        fixture.write("target_policy.json", old_policy.as_bytes());
        assert!(target_policy::authorize(
            &fixture.startup().target_policy,
            &peer.fingerprint,
            "127.0.0.1:12345"
        )
        .await
        .is_ok());

        fixture.write(
            "target_policy.json",
            br#"{"mode":"deny-by-default","peers":{}}"#,
        );
        assert!(target_policy::authorize(
            &fixture.startup().target_policy,
            &peer.fingerprint,
            "127.0.0.1:12345"
        )
        .await
        .is_err());

        fixture.write("target_policy.json", old_policy.as_bytes());
        assert!(target_policy::authorize(
            &fixture.startup().target_policy,
            &peer.fingerprint,
            "127.0.0.1:12345"
        )
        .await
        .is_ok());
    }

    #[test]
    fn characterizes_old_pelt_and_active_fang_state_restoration() {
        let fixture = Fixture::new();
        let old_pelt = generate_identity();
        let new_pelt = generate_identity();
        let remote_pelt = generate_identity();
        let remote_peer = PeerRecord {
            name: "former-peer".into(),
            fingerprint: remote_pelt.fingerprint.clone(),
            address: "tcp://127.0.0.1:12345".into(),
            trust: TrustLevel::Packmate,
            public_key_b64: Some(remote_pelt.public_key_b64.clone()),
        };
        let old_pack = serde_json::to_vec(&vec![remote_peer]).unwrap();
        let old_pelt_json = serde_json::to_vec(&old_pelt).unwrap();
        fixture.write("pelt.json", &old_pelt_json);
        fixture.write("pack.json", &old_pack);
        let old_fangs = br#"[{"name":"old-route","peer":"former-peer","local":"127.0.0.1:12346","remote":"127.0.0.1:12345","transport":"tcp"}]"#;
        fixture.write("fangs.json", old_fangs);
        fixture.write("active_fangs.json", br#"["old-route"]"#);
        assert_eq!(fixture.startup().active_profiles, ["old-route"]);

        fixture.write("pelt.json", &serde_json::to_vec(&new_pelt).unwrap());
        fixture.write("pack.json", b"[]");
        fixture.write("active_fangs.json", b"[]");
        fixture.write("fangs.json", b"[]");
        let mut unrelated = fixture.startup();
        assert_eq!(
            unrelated.pelt.as_ref().unwrap().fingerprint,
            new_pelt.fingerprint
        );
        unrelated.initialize_runtime_tls_identity().unwrap();
        let unrelated_tls = unrelated.runtime_tls_identity.as_ref().unwrap();
        let unrelated_cert = webpki::EndEntityCert::try_from(unrelated_tls.certificate()).unwrap();
        assert_eq!(
            unrelated_cert.subject_public_key_info().as_ref(),
            crate::tls_identity::canonical_spki_from_public_key_b64(&new_pelt.public_key_b64)
                .unwrap()
        );
        assert!(unrelated.active_profiles.is_empty());

        fixture.write("pelt.json", &old_pelt_json);
        fixture.write("pack.json", &old_pack);
        fixture.write("fangs.json", old_fangs);
        fixture.write("active_fangs.json", br#"["old-route"]"#);
        let mut restored = fixture.startup();
        assert_eq!(
            restored.pelt.as_ref().unwrap().fingerprint,
            old_pelt.fingerprint
        );
        restored.initialize_runtime_tls_identity().unwrap();
        let restored_tls = restored.runtime_tls_identity.as_ref().unwrap();
        let restored_cert = webpki::EndEntityCert::try_from(restored_tls.certificate()).unwrap();
        assert_eq!(
            restored_cert.subject_public_key_info().as_ref(),
            crate::tls_identity::canonical_spki_from_public_key_b64(&old_pelt.public_key_b64)
                .unwrap()
        );
        assert_eq!(restored.active_profiles, ["old-route"]);
        assert_eq!(restored.peers[0].name, "former-peer");
        assert_eq!(restored.fang_profiles[0].remote, "127.0.0.1:12345");
    }

    #[test]
    fn characterizes_mixed_and_whole_den_valid_snapshot_restoration() {
        let fixture = Fixture::new();
        let old_pelt = generate_identity();
        let new_pelt = generate_identity();
        let old_pelt_json = serde_json::to_vec(&old_pelt).unwrap();
        fixture.write("pelt.json", &old_pelt_json);
        fixture.write("pack.json", b"[]");
        fixture.write("target_policy.json", br#"{"mode":"legacy-allow"}"#);
        fixture.write("fangs.json", b"[]");
        fixture.write("active_fangs.json", b"[]");
        assert!(!fixture.startup().inbound_authority.is_locked());
        let old_snapshot: Vec<_> = [
            "silver.json",
            "target_policy.json",
            "pelt.json",
            "pack.json",
            "fangs.json",
            "active_fangs.json",
        ]
        .into_iter()
        .map(|name| {
            (
                name,
                Zeroizing::new(
                    fixture
                        .1
                        .read(OsStr::new(name), 1024 * 1024)
                        .unwrap()
                        .unwrap(),
                ),
            )
        })
        .collect();

        fixture.write("pelt.json", &serde_json::to_vec(&new_pelt).unwrap());
        fixture.write("silver.json", br#"{"version":1,"mode":"locked"}"#);
        fixture.write(
            "target_policy.json",
            br#"{"mode":"deny-by-default","peers":{}}"#,
        );
        assert!(fixture.startup().inbound_authority.is_locked());

        // A cross-generation set of individually valid files is accepted.
        fixture.write("target_policy.json", br#"{"mode":"legacy-allow"}"#);
        let mixed = fixture.startup();
        assert!(mixed.inbound_authority.is_locked());
        assert!(matches!(
            mixed.target_policy,
            target_policy::TargetPolicy::LegacyAllow
        ));
        assert_eq!(mixed.pelt.unwrap().fingerprint, new_pelt.fingerprint);

        // Restoring every protected document from the earlier valid state
        // gives startup no persistent freshness evidence to reject it.
        for (name, old_bytes) in &old_snapshot {
            fixture.write(name, old_bytes);
        }
        let restored = fixture.startup();
        assert!(!restored.inbound_authority.is_locked());
        assert!(matches!(
            restored.target_policy,
            target_policy::TargetPolicy::LegacyAllow
        ));
        assert_eq!(restored.pelt.unwrap().fingerprint, old_pelt.fingerprint);

        // Existing semantic validation catches a dangling active profile,
        // but this is not a general cross-file revision check.
        fixture.write("active_fangs.json", br#"["absent"]"#);
        assert!(load_startup_state(&fixture.1, &mut DaemonState::default()).is_err());
    }

    #[test]
    fn characterizes_named_mixed_generation_combinations() {
        let fixture = Fixture::new();
        let old_pelt = generate_identity();
        let new_pelt = generate_identity();
        let old_peer_identity = generate_identity();
        let new_peer_identity = generate_identity();
        let peer = |name: &str, identity: &werewolf_core::pelt::PeltIdentity| PeerRecord {
            name: name.into(),
            fingerprint: identity.fingerprint.clone(),
            address: "tcp://127.0.0.1:12345".into(),
            trust: TrustLevel::Packmate,
            public_key_b64: Some(identity.public_key_b64.clone()),
        };
        let old_pack = serde_json::to_vec(&vec![peer("old-peer", &old_peer_identity)]).unwrap();
        let new_pack = serde_json::to_vec(&vec![peer("new-peer", &new_peer_identity)]).unwrap();
        let old_policy = serde_json::json!({
            "mode": "deny-by-default",
            "peers": {old_peer_identity.fingerprint.clone(): {"targets": [{"address": "127.0.0.1", "port": 12345}]}}
        })
        .to_string();
        let new_policy = serde_json::json!({
            "mode": "deny-by-default",
            "peers": {new_peer_identity.fingerprint.clone(): {"targets": [{"address": "127.0.0.1", "port": 12346}]}}
        })
        .to_string();
        let old_fangs = br#"[{"name":"old-route","peer":"old-peer","local":"127.0.0.1:12347","remote":"127.0.0.1:12345","transport":"tcp"}]"#;
        let new_fangs = br#"[{"name":"new-route","peer":"new-peer","local":"127.0.0.1:12348","remote":"127.0.0.1:12346","transport":"tcp"}]"#;

        fixture.write("pelt.json", &serde_json::to_vec(&old_pelt).unwrap());
        fixture.write("pack.json", &old_pack);
        fixture.write("target_policy.json", old_policy.as_bytes());
        fixture.write("fangs.json", old_fangs);
        fixture.write("active_fangs.json", br#"["old-route"]"#);

        fixture.write("pelt.json", &serde_json::to_vec(&new_pelt).unwrap());
        fixture.write("pack.json", &new_pack);
        fixture.write("target_policy.json", new_policy.as_bytes());
        fixture.write("fangs.json", new_fangs);
        fixture.write("active_fangs.json", br#"["new-route"]"#);
        fixture.write("silver.json", br#"{"version":1,"mode":"locked"}"#);

        // old Pack + new Silver: each valid file loads; Silver stays a
        // runtime fence even while historic Pack membership returns.
        fixture.write("pack.json", &old_pack);
        let old_pack_new_silver = fixture.startup();
        assert!(old_pack_new_silver.inbound_authority.is_locked());
        assert_eq!(old_pack_new_silver.peers[0].name, "old-peer");

        // new Pack + old target policy and old Pelt + newer Pack are both
        // individually valid mixed generations; no common revision rejects
        // them. Target policy remains an independent authorization layer.
        fixture.write("pack.json", &new_pack);
        fixture.write("target_policy.json", old_policy.as_bytes());
        fixture.write("pelt.json", &serde_json::to_vec(&old_pelt).unwrap());
        let new_pack_old_policy_old_pelt = fixture.startup();
        assert_eq!(new_pack_old_policy_old_pelt.peers[0].name, "new-peer");
        assert_eq!(
            new_pack_old_policy_old_pelt.pelt.unwrap().fingerprint,
            old_pelt.fingerprint
        );
        assert!(matches!(
            new_pack_old_policy_old_pelt.target_policy,
            target_policy::TargetPolicy::Grants(_)
        ));

        // A new Fang registry with an old active list is rejected only when
        // the old profile name is absent. This is the existing referential
        // check, not a general generation check.
        fixture.write("fangs.json", new_fangs);
        fixture.write("active_fangs.json", br#"["old-route"]"#);
        assert!(load_startup_state(&fixture.1, &mut DaemonState::default()).is_err());
    }
}
