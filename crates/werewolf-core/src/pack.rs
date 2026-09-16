use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrustLevel {
    Stranger,
    Known,
    Packmate,
    Alpha,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerRecord {
    pub name: String,
    pub fingerprint: String,
    pub address: String,
    pub trust: TrustLevel,

    #[serde(default)]
    pub public_key_b64: Option<String>,
}

pub fn load_pack(path: &Path) -> io::Result<Vec<PeerRecord>> {
    let peers: Vec<PeerRecord> = crate::state_file::read(path)?.unwrap_or_default();
    crate::state_validation::pack(&peers)?;
    Ok(peers)
}

pub fn save_pack(path: &Path, peers: &[PeerRecord]) -> io::Result<()> {
    crate::state_validation::pack(peers)?;
    crate::state_file::write(path, peers, false)
}
