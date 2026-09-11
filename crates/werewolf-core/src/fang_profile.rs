use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

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
    if !path.exists() {
        return Ok(vec![]);
    }

    let data = fs::read_to_string(path)?;
    let profiles: Vec<FangProfile> = serde_json::from_str(&data)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    Ok(profiles)
}

pub fn save_fang_profiles(path: &Path, profiles: &[FangProfile]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let data = serde_json::to_string_pretty(profiles)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    fs::write(path, data)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}
