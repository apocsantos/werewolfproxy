use crate::{state::DaemonState, target_policy};
use werewolf_core::{fang_profile::load_fang_profiles, pack::load_pack, pelt::load_identity};

pub(super) fn load_startup_state(home: &std::path::Path, initial_state: &mut DaemonState) {
    initial_state.target_policy = target_policy::load(&home.join("target_policy.json"));
    let pelt_path = home.join("pelt.json");
    match load_identity(&pelt_path) {
        Ok(Some(identity)) => {
            ww_info!("PELT", "LOADED", "🐾 Pelt loaded: {}", identity.fingerprint);
            initial_state.status.pelt_ready = true;
            initial_state.pelt = Some(identity);
        }
        Ok(None) => {
            ww_warn!(
                "PELT",
                "MISSING",
                "🐾 No Pelt found. Run: werewolfctl pelt init"
            );
        }
        Err(e) => {
            ww_warn!("PELT", "LOAD_FAILED", "⚠️ Failed to load Pelt: {}", e);
        }
    }

    let pack_path = home.join("pack.json");
    match load_pack(&pack_path) {
        Ok(peers) => {
            ww_info!("PACK", "LOADED", "🐾 Loaded {} packmates", peers.len());
            initial_state.peers = peers;
        }
        Err(e) => {
            ww_warn!("PACK", "LOAD_FAILED", "⚠️ Failed to load Pack: {}", e);
        }
    }

    let fang_profiles_path = home.join("fangs.json");
    match load_fang_profiles(&fang_profiles_path) {
        Ok(profiles) => {
            ww_info!(
                "FANG",
                "PROFILES_LOADED",
                "🦷 Loaded {} Fang profiles",
                profiles.len()
            );
            initial_state.fang_profiles = profiles;
        }
        Err(e) => {
            ww_warn!(
                "FANG",
                "PROFILES_LOAD_FAILED",
                "⚠️ Failed to load Fang profiles: {}",
                e
            );
        }
    }
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
