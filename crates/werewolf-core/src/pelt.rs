use base64::{engine::general_purpose::STANDARD, Engine};
use blake3::Hasher;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeltIdentity {
    pub public_key_b64: String,
    pub secret_key_b64: String,
    pub fingerprint: String,
}

impl std::fmt::Debug for PeltIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeltIdentity")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
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

/// Initialization only. Existing identity state is never replaced.
pub fn save_identity(path: &Path, identity: &PeltIdentity) -> io::Result<()> {
    crate::state_validation::identity(identity)?;
    crate::state_file::write(path, identity, true)
}

pub fn load_identity(path: &Path) -> io::Result<Option<PeltIdentity>> {
    let identity = crate::state_file::read(path)?;
    if let Some(identity) = &identity {
        crate::state_validation::identity(identity)?;
    }
    Ok(identity)
}

pub fn sign_message(identity: &PeltIdentity, message: &[u8]) -> Result<String, String> {
    let secret_bytes = STANDARD
        .decode(&identity.secret_key_b64)
        .map_err(|e| e.to_string())?;

    let secret: [u8; 32] = secret_bytes
        .try_into()
        .map_err(|_| "invalid secret key length".to_string())?;

    let signing_key = SigningKey::from_bytes(&secret);
    let sig = signing_key.sign(message);

    Ok(STANDARD.encode(sig.to_bytes()))
}

pub fn verify_message(
    public_key_b64: &str,
    message: &[u8],
    signature_b64: &str,
) -> Result<(), String> {
    let public_bytes = STANDARD.decode(public_key_b64).map_err(|e| e.to_string())?;

    let public: [u8; 32] = public_bytes
        .try_into()
        .map_err(|_| "invalid public key length".to_string())?;

    let verifying_key = VerifyingKey::from_bytes(&public).map_err(|e| e.to_string())?;

    let sig_bytes = STANDARD.decode(signature_b64).map_err(|e| e.to_string())?;

    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| "invalid signature length".to_string())?;

    let sig = Signature::from_bytes(&sig_arr);

    verifying_key
        .verify(message, &sig)
        .map_err(|e| e.to_string())
}

pub fn fingerprint_from_public_key_b64(public_key_b64: &str) -> Result<String, String> {
    let public = STANDARD.decode(public_key_b64).map_err(|e| e.to_string())?;

    let mut hasher = Hasher::new();
    hasher.update(&public);
    let hash = hasher.finalize();

    let fp_hex = hexish(&hash.as_bytes()[0..8]);
    Ok(format!("wwp1:{}", fp_hex))
}

fn hexish(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join("-")
}
