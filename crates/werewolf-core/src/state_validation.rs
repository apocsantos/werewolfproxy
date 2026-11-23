//! Semantic validation of persistent identity/trust documents. No network I/O.
use crate::{
    fang_profile::FangProfile,
    pack::PeerRecord,
    pelt::{fingerprint_from_public_key_b64, PeltIdentity},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::SigningKey;
use std::{collections::HashSet, io};
use zeroize::Zeroizing;

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid persistent security state",
    )
}
fn key(text: &str) -> io::Result<[u8; 32]> {
    let bytes = STANDARD.decode(text).map_err(|_| invalid())?;
    if STANDARD.encode(&bytes) != text {
        return Err(invalid());
    }
    bytes.try_into().map_err(|_| invalid())
}
pub fn fingerprint(text: &str) -> bool {
    let Some(hex) = text.strip_prefix("wwp1:") else {
        return false;
    };
    let parts: Vec<_> = hex.split('-').collect();
    parts.len() == 8
        && parts.iter().all(|p| {
            p.len() == 2
                && p.bytes()
                    .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
        })
}
pub fn identity(identity: &PeltIdentity) -> io::Result<()> {
    let public = key(&identity.public_key_b64)?;
    let decoded = Zeroizing::new(
        STANDARD
            .decode(&identity.secret_key_b64)
            .map_err(|_| invalid())?,
    );
    if STANDARD.encode(&decoded[..]) != identity.secret_key_b64 {
        return Err(invalid());
    }
    let secret: Zeroizing<[u8; 32]> =
        Zeroizing::new(decoded.as_slice().try_into().map_err(|_| invalid())?);
    if SigningKey::from_bytes(&secret).verifying_key().to_bytes() != public
        || !fingerprint(&identity.fingerprint)
        || fingerprint_from_public_key_b64(&identity.public_key_b64).map_err(|_| invalid())?
            != identity.fingerprint
    {
        return Err(invalid());
    }
    Ok(())
}
fn name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text.trim() == text
        && !text.chars().any(char::is_control)
}
pub fn endpoint(text: &str) -> bool {
    if text.is_empty()
        || text.len() > 512
        || text.chars().any(char::is_whitespace)
        || text.chars().any(char::is_control)
    {
        return false;
    }
    let Some((host, port)) = text.rsplit_once(':') else {
        return false;
    };
    !host.is_empty() && port.parse::<u16>().is_ok_and(|p| p != 0)
}

/// Local Fang bind syntax: the legacy TCP socket address, or a Linux
/// filesystem Unix socket explicitly tagged with `unix:`.
pub fn local_fang_endpoint(text: &str) -> bool {
    if text.parse::<std::net::SocketAddr>().is_ok() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        let Some(path) = text.strip_prefix("unix:") else {
            return false;
        };
        let path = std::path::Path::new(path);
        path.is_absolute()
            && path.file_name().is_some()
            && !path
                .as_os_str()
                .to_string_lossy()
                .split('/')
                .any(|part| part == "." || part == "..")
            && !text.chars().any(char::is_whitespace)
            && !text.chars().any(char::is_control)
            && text.len() <= 512
    }
    #[cfg(not(target_os = "linux"))]
    false
}
pub fn pack(peers: &[PeerRecord]) -> io::Result<()> {
    if peers.len() > 1024 {
        return Err(invalid());
    }
    let (mut names, mut fingerprints, mut keys) = (HashSet::new(), HashSet::new(), HashSet::new());
    for peer in peers {
        let address = peer
            .address
            .strip_prefix("tcp://")
            .or_else(|| peer.address.strip_prefix("quic://"))
            .ok_or_else(invalid)?;
        if !name(&peer.name)
            || !fingerprint(&peer.fingerprint)
            || !endpoint(address)
            || !names.insert(&peer.name)
            || !fingerprints.insert(&peer.fingerprint)
        {
            return Err(invalid());
        }
        if let Some(public) = &peer.public_key_b64 {
            let bytes = key(public)?;
            if ed25519_dalek::VerifyingKey::from_bytes(&bytes).is_err()
                || !keys.insert(bytes)
                || fingerprint_from_public_key_b64(public).map_err(|_| invalid())?
                    != peer.fingerprint
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
pub fn profiles(profiles: &[FangProfile], peers: &[PeerRecord]) -> io::Result<()> {
    profile_document(profiles)?;
    if profiles
        .iter()
        .any(|p| !peers.iter().any(|peer| peer.name == p.peer))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Validate stored profile syntax independently of current Pack membership.
/// A removed peer leaves an inert profile; opening still requires current authority.
pub fn profile_document(profiles: &[FangProfile]) -> io::Result<()> {
    if profiles.len() > 512 {
        return Err(invalid());
    }
    let mut names = HashSet::new();
    for p in profiles {
        if !name(&p.name)
            || !names.insert(&p.name)
            || !name(&p.peer)
            || !local_fang_endpoint(&p.local)
            || !endpoint(&p.remote)
            || !matches!(p.transport.as_str(), "quic" | "tcp" | "tcp-plain")
        {
            return Err(invalid());
        }
    }
    Ok(())
}
pub fn active(active: &[String], profiles: &[FangProfile]) -> io::Result<()> {
    let mut names = HashSet::new();
    if active.len() > 512
        || active
            .iter()
            .any(|name| !names.insert(name) || !profiles.iter().any(|p| p.name == *name))
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pack::TrustLevel, pelt::generate_identity};
    #[test]
    fn identity_consistency_and_canonical_encoding() {
        let good = generate_identity();
        identity(&good).unwrap();
        let mut bad = good.clone();
        bad.fingerprint = generate_identity().fingerprint.clone();
        assert!(identity(&bad).is_err());
        let mut bad = good.clone();
        bad.public_key_b64 = generate_identity().public_key_b64.clone();
        assert!(identity(&bad).is_err());
        let mut bad = good.clone();
        bad.secret_key_b64 = "invalid".into();
        assert!(identity(&bad).is_err());
        assert!(!format!("{good:?}").contains(&good.secret_key_b64));
    }
    #[test]
    fn duplicate_conflicting_and_malformed_pack_rejected() {
        let pelt = generate_identity();
        let peer = PeerRecord {
            name: "peer".into(),
            fingerprint: pelt.fingerprint.clone(),
            address: "quic://127.0.0.1:1".into(),
            trust: TrustLevel::Packmate,
            public_key_b64: Some(pelt.public_key_b64.clone()),
        };
        pack(std::slice::from_ref(&peer)).unwrap();
        assert!(pack(&[peer.clone(), peer.clone()]).is_err());
        let mut other = peer.clone();
        other.name = "alias".into();
        assert!(pack(&[peer.clone(), other]).is_err());
        let mut bad = peer.clone();
        bad.fingerprint = "wwp1:bad".into();
        assert!(pack(&[bad]).is_err());
        let mut bad = peer;
        bad.public_key_b64 = Some(generate_identity().public_key_b64.clone());
        assert!(pack(&[bad]).is_err());
    }

    #[test]
    fn local_fang_endpoint_keeps_tcp_and_accepts_only_safe_linux_unix_syntax() {
        assert!(local_fang_endpoint("127.0.0.1:1234"));
        #[cfg(target_os = "linux")]
        {
            assert!(local_fang_endpoint("unix:/run/werewolf/fang.sock"));
            assert!(!local_fang_endpoint("unix:relative/fang.sock"));
            assert!(!local_fang_endpoint("unix:/run/../fang.sock"));
            assert!(!local_fang_endpoint("unix:/run/./fang.sock"));
        }
        #[cfg(not(target_os = "linux"))]
        assert!(!local_fang_endpoint("unix:/run/werewolf/fang.sock"));
    }
}
