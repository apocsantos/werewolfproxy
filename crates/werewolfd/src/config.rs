use crate::{cli::Args, state::DaemonState};
use std::path::PathBuf;
use werewolf_core::pelt::fingerprint_from_public_key_b64;

pub(super) fn configure_den(initial_state: &mut DaemonState, args: &Args, home: &std::path::Path) {
    initial_state.den_socket = args.socket.clone();
    initial_state.den_home = home.display().to_string();
    initial_state.den_listen = args.listen.clone();
    initial_state.den_quic_listen = args.quic_listen.clone();
}

pub(super) fn expand_home(input: &str) -> PathBuf {
    if input == "~" {
        return PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));
    }

    if let Some(rest) = input.strip_prefix("~/") {
        return PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string())).join(rest);
    }

    PathBuf::from(input)
}

pub(super) fn validate_startup_config(state: &DaemonState) {
    ww_info!(
        "CONFIG",
        "VALIDATE_START",
        "🧪 Validating startup config..."
    );

    for peer in &state.peers {
        if !peer.fingerprint.starts_with("wwp1:") {
            eprintln!(
                "⚠️ Config warning: peer {} has invalid fingerprint {}",
                peer.name, peer.fingerprint
            );
        }

        if !(peer.address.starts_with("tcp://") || peer.address.starts_with("quic://")) {
            eprintln!(
                "⚠️ Config warning: peer {} has invalid address {}",
                peer.name, peer.address
            );
        }

        if peer.address.starts_with("quic://") && peer.public_key_b64.is_none() {
            eprintln!(
                "⚠️ Config warning: QUIC peer {} has no public_key_b64",
                peer.name
            );
        }

        if let Some(pk) = &peer.public_key_b64 {
            match fingerprint_from_public_key_b64(pk) {
                Ok(fp) => {
                    if fp != peer.fingerprint {
                        eprintln!(
                            "⚠️ Config warning: peer {} public key fingerprint mismatch: expected {}, got {}",
                            peer.name,
                            peer.fingerprint,
                            fp
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "⚠️ Config warning: peer {} has invalid public_key_b64: {}",
                        peer.name, e
                    );
                }
            }
        }
    }

    for profile in &state.fang_profiles {
        if profile.name.trim().is_empty() {
            eprintln!("⚠️ Config warning: Fang profile with empty name");
        }

        if profile.peer.trim().is_empty() {
            eprintln!(
                "⚠️ Config warning: Fang profile {} has empty peer",
                profile.name
            );
        }

        if profile.local.parse::<std::net::SocketAddr>().is_err() {
            eprintln!(
                "⚠️ Config warning: Fang profile {} has invalid local address {}",
                profile.name, profile.local
            );
        }

        if profile.remote.parse::<std::net::SocketAddr>().is_err() {
            eprintln!(
                "⚠️ Config warning: Fang profile {} has invalid remote address {}",
                profile.name, profile.remote
            );
        }

        if !state.peers.iter().any(|p| p.name == profile.peer) {
            eprintln!(
                "⚠️ Config warning: Fang profile {} references unknown peer {}",
                profile.name, profile.peer
            );
        }
    }

    ww_info!(
        "CONFIG",
        "VALIDATE_OK",
        "✅ Startup config validation complete"
    );
}
