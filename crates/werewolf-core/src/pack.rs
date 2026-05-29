use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrustLevel {
    Stranger,
    Known,
    Packmate,
    Alpha,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRecord {
    pub name: String,
    pub fingerprint: String,
    pub address: String,
    pub trust: TrustLevel,

    #[serde(default)]
    pub public_key_b64: Option<String>,
}

pub fn load_pack(path: &Path) -> io::Result<Vec<PeerRecord>> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let data = fs::read_to_string(path)?;
    let peers: Vec<PeerRecord> = serde_json::from_str(&data)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    Ok(peers)
}

pub fn save_pack(path: &Path, peers: &[PeerRecord]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let data = serde_json::to_string_pretty(peers)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    fs::write(path, data)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}
