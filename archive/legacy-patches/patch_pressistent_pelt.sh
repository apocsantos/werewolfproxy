#!/usr/bin/env bash
set -e

cat > crates/werewolf-core/src/pelt.rs <<'EOF'
use base64::{engine::general_purpose::STANDARD, Engine};
use blake3::Hasher;
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeltIdentity {
    pub public_key_b64: String,
    pub secret_key_b64: String,
    pub fingerprint: String,
}

pub fn generate_identity() -> PeltIdentity {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let public = verifying_key.to_bytes();
    let secret = signing_key.to_bytes();

    let mut hasher = Hasher::new();
    hasher.update(&public);
    let hash = hasher.finalize();

    let fp_hex = hexish(&hash.as_bytes()[0..8]);
    let fingerprint = format!("wwp1:{}", fp_hex);

    PeltIdentity {
        public_key_b64: STANDARD.encode(public),
        secret_key_b64: STANDARD.encode(secret),
        fingerprint,
    }
}

pub fn save_identity(path: &Path, identity: &PeltIdentity) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let data = serde_json::to_string_pretty(identity)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    fs::write(path, data)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

pub fn load_identity(path: &Path) -> io::Result<Option<PeltIdentity>> {
    if !path.exists() {
        return Ok(None);
    }

    let data = fs::read_to_string(path)?;
    let identity: PeltIdentity = serde_json::from_str(&data)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    Ok(Some(identity))
}

fn hexish(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join("-")
}
EOF

python3 - <<'PY'
from pathlib import Path
p = Path("crates/werewolfd/src/main.rs")
s = p.read_text()

s = s.replace(
'use werewolf_core::{\n    pelt::{generate_identity, PeltIdentity},',
'use werewolf_core::{\n    pelt::{generate_identity, load_identity, save_identity, PeltIdentity},'
)

s = s.replace(
'const SOCKET_PATH: &str = "/tmp/werewolf.sock";',
'const SOCKET_PATH: &str = "/tmp/werewolf.sock";\nconst PELT_PATH: &str = ".config/werewolf/pelt.json";'
)

s = s.replace(
'let state = Arc::new(Mutex::new(DaemonState::default()));',
'''let mut initial_state = DaemonState::default();

    let pelt_path = home_pelt_path();
    match load_identity(&pelt_path) {
        Ok(Some(identity)) => {
            println!("🐾 Pelt loaded: {}", identity.fingerprint);
            initial_state.status.pelt_ready = true;
            initial_state.pelt = Some(identity);
        }
        Ok(None) => {
            println!("🐾 No Pelt found. Run: werewolfctl pelt init");
        }
        Err(e) => {
            eprintln!("⚠️ Failed to load Pelt: {}", e);
        }
    }

    let state = Arc::new(Mutex::new(initial_state));'''
)

s = s.replace(
'''let identity = generate_identity();
            st.status.pelt_ready = true;
            st.pelt = Some(identity.clone());
            ControlResponse::ok(req.id, json!({
                "fingerprint": identity.fingerprint
            }))''',
'''let identity = generate_identity();
            let pelt_path = home_pelt_path();

            match save_identity(&pelt_path, &identity) {
                Ok(_) => {
                    st.status.pelt_ready = true;
                    st.pelt = Some(identity.clone());
                    ControlResponse::ok(req.id, json!({
                        "fingerprint": identity.fingerprint,
                        "saved_to": pelt_path
                    }))
                }
                Err(e) => ControlResponse::err(req.id, "PELT_SAVE_FAILED", e.to_string()),
            }'''
)

s = s.replace(
'''// tiny local alias so we avoid adding anyhow yet
mod anyhow_free {''',
'''fn home_pelt_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(PELT_PATH)
}

// tiny local alias so we avoid adding anyhow yet
mod anyhow_free {'''
)

p.write_text(s)
PY

echo "🐾 Persistent Pelt patch applied."
