use crate::{state::DaemonState, target_policy};
use std::{ffi::OsStr, io};
use werewolf_core::{local_fs::PrivateDirectory, state_validation};

pub(super) fn load_startup_state(
    den: &PrivateDirectory,
    initial_state: &mut DaemonState,
) -> io::Result<()> {
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
        state_validation::profiles(&profiles, &initial_state.peers)?;
        initial_state.fang_profiles = profiles;
    }
    Ok(())
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid persistent security state",
    )
}

pub(super) fn load_active_fang_profiles(path: &std::path::Path) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_active_fang_profiles(path: &std::path::Path, profiles: &[String]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let data = serde_json::to_string_pretty(profiles)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    std::fs::write(path, data)
}

pub(super) fn remember_active_fang_profile(home: &std::path::Path, profile_name: &str) {
    let path = home.join("active_fangs.json");
    let mut profiles = load_active_fang_profiles(&path);

    if !profiles.iter().any(|p| p == profile_name) {
        profiles.push(profile_name.to_string());
    }

    if let Err(e) = save_active_fang_profiles(&path, &profiles) {
        eprintln!(
            "⚠️ Failed to persist active Fang profile {}: {}",
            profile_name, e
        );
    }
}

pub(super) fn forget_active_fang_profile(home: &std::path::Path, profile_name: &str) {
    let path = home.join("active_fangs.json");
    let mut profiles = load_active_fang_profiles(&path);

    profiles.retain(|p| p != profile_name);

    if let Err(e) = save_active_fang_profiles(&path, &profiles) {
        eprintln!("⚠️ Failed to update active Fang profiles: {}", e);
    }
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
}
