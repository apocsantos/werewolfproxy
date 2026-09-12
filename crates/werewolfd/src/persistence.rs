use crate::{authority::Authority, state::DaemonState, target_policy};
use std::{ffi::OsStr, io};
use werewolf_core::{local_fs::PrivateDirectory, state_validation};

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
    initial_state.inbound_authority = Authority::new(silver_locked);
    initial_state.status.silver = if silver_locked {
        "active".into()
    } else {
        "armed".into()
    };
    initial_state.status.mode = if silver_locked {
        werewolf_core::state::WolfMode::Silver
    } else {
        werewolf_core::state::WolfMode::Human
    };
    initial_state.target_policy = match den.read(OsStr::new("target_policy.json"), 1024 * 1024)? {
        None => target_policy::TargetPolicy::Deny,
        Some(data) => match std::str::from_utf8(&data) {
            Ok(text) => target_policy::parse(text),
            Err(_) => target_policy::TargetPolicy::Invalid,
        },
    };
    if let Some(data) = den.read(OsStr::new("pelt.json"), 4096)? {
        let pelt = serde_json::from_slice(&data).map_err(|_| invalid())?;
        state_validation::identity(&pelt)?;
        initial_state.pelt = Some(pelt);
        initial_state.status.pelt_ready = true;
    }
    if let Some(data) = den.read(OsStr::new("pack.json"), 1024 * 1024)? {
        let peers = serde_json::from_slice::<Vec<_>>(&data).map_err(|_| invalid())?;
        state_validation::pack(&peers)?;
        initial_state.peers = peers;
    }
    if let Some(data) = den.read(OsStr::new("fangs.json"), 1024 * 1024)? {
        let profiles = serde_json::from_slice::<Vec<_>>(&data).map_err(|_| invalid())?;
        state_validation::profile_document(&profiles)?;
        initial_state.fang_profiles = profiles;
    }
    if let Some(data) = den.read(OsStr::new("active_fangs.json"), 1024 * 1024)? {
        let active = serde_json::from_slice::<Vec<String>>(&data).map_err(|_| invalid())?;
        state_validation::active(&active, &initial_state.fang_profiles)?;
        initial_state.active_profiles = active;
    }
    Ok(())
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
    use werewolf_core::{local_fs::CommitOutcome, pelt::generate_identity};
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
}
