use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FangProfile {
    pub name: String,
    pub peer: String,
    pub local: String,
    pub remote: String,

    #[serde(default = "default_transport")]
    pub transport: String,
}

fn default_transport() -> String {
    "quic".to_string()
}

pub fn load_fang_profiles(path: &Path) -> io::Result<Vec<FangProfile>> {
    let profiles: Vec<FangProfile> = crate::state_file::read(path)?.unwrap_or_default();
    crate::state_validation::profile_document(&profiles)?;
    Ok(profiles)
}

pub fn save_fang_profiles(path: &Path, profiles: &[FangProfile]) -> io::Result<()> {
    crate::state_validation::profile_document(profiles)?;
    crate::state_file::write(path, profiles, false)
}
