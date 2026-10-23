//! The offline governance tool (Stage28 survey decision 2a): key files,
//! drafts, signature parts and combined statements, as files. Nothing here
//! talks to a daemon or the network.
//!
//! - Key file: JSON `{version, type, public_b64, secret_b64}`, created new
//!   with mode 0600 and refused when readable by anyone but its owner.
//! - Draft: a statement with no signatures, in the spec's encoding.
//! - Part: JSON `{version, hash, index, signature}`; `hash` names the draft
//!   it signs, so a part can never be combined into another statement.
//! - Statement: the spec's encoding.

use super::{apply, sign, Body, Core, GovError, Hash, Head, Key, Outcome, Statement};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::SigningKey;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
use zeroize::Zeroizing;

const KEY_TYPE: &str = "werewolf-governance-key";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    version: u8,
    r#type: String,
    public_b64: String,
    secret_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub version: u8,
    pub hash: String,
    pub index: u8,
    pub signature: String,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn gov(error: GovError) -> io::Error {
    invalid(error.to_string())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex<const N: usize>(text: &str) -> io::Result<[u8; N]> {
    if text.len() != 2 * N || !text.is_ascii() {
        return Err(invalid("expected {N} bytes of hex"));
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte =
            u8::from_str_radix(&text[2 * i..2 * i + 2], 16).map_err(|_| invalid("expected hex"))?;
    }
    Ok(out)
}

pub fn encode_b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn public_key(b64: &str) -> io::Result<Key> {
    STANDARD
        .decode(b64)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| invalid("expected a base64 Ed25519 public key"))
}

/// Writes `bytes` to a new file; never replaces an existing one.
fn create(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Generates one holder's key and writes it to `path` (mode 0600). Returns
/// the public key, which is what the genesis draft needs.
pub fn keygen(path: &Path) -> io::Result<String> {
    let key = SigningKey::generate(&mut OsRng);
    let public_b64 = STANDARD.encode(key.verifying_key().to_bytes());
    let secret = Zeroizing::new(STANDARD.encode(key.to_bytes()));
    let file = KeyFile {
        version: 1,
        r#type: KEY_TYPE.into(),
        public_b64: public_b64.clone(),
        secret_b64: secret.to_string(),
    };
    let bytes = Zeroizing::new(serde_json::to_vec_pretty(&file).map_err(invalid_json)?);
    create(path, &bytes, 0o600)?;
    Ok(public_b64)
}

fn invalid_json(error: serde_json::Error) -> io::Error {
    invalid(error.to_string())
}

fn read_key(path: &Path) -> io::Result<SigningKey> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if fs::metadata(path)?.mode() & 0o077 != 0 {
            return Err(invalid(
                "governance key file must be readable by its owner only",
            ));
        }
    }
    let bytes = Zeroizing::new(fs::read(path)?);
    let file: KeyFile = serde_json::from_slice(&bytes).map_err(invalid_json)?;
    if file.version != 1 || file.r#type != KEY_TYPE {
        return Err(invalid("not a governance key file"));
    }
    let secret = Zeroizing::new(
        STANDARD
            .decode(&file.secret_b64)
            .map_err(|_| invalid("bad secret encoding"))?,
    );
    let secret: &[u8; 32] = secret
        .as_slice()
        .try_into()
        .map_err(|_| invalid("bad secret length"))?;
    let key = SigningKey::from_bytes(secret);
    if STANDARD.encode(key.verifying_key().to_bytes()) != file.public_b64 {
        return Err(invalid("key file public key does not match its secret"));
    }
    Ok(key)
}

pub fn read_statement(path: &Path) -> io::Result<Statement> {
    Statement::decode(&fs::read(path)?).map_err(gov)
}

pub fn write_statement(path: &Path, statement: &Statement) -> io::Result<()> {
    create(path, &statement.encode(), 0o644)
}

fn unsigned(core: Core) -> Statement {
    Statement {
        core,
        signatures: Vec::new(),
    }
}

/// A genesis draft for the ten public keys in index order. A random network
/// ID unless one is given.
pub fn draft_genesis(keys: &[Key], network_id: Option<[u8; 32]>) -> Statement {
    let network_id = network_id.unwrap_or_else(|| {
        let mut id = [0u8; 32];
        OsRng.fill_bytes(&mut id);
        id
    });
    unsigned(Core {
        network_id,
        sequence: 0,
        previous: [0; 32],
        body: Body::Genesis {
            threshold: super::THRESHOLD as u8,
            keys: keys.to_vec(),
        },
    })
}

/// A draft following `previous`, the current chain head's statement.
pub fn draft_after(previous: &Statement, body: Body) -> Statement {
    let body = match body {
        Body::Revocation(mut keys) => {
            keys.sort();
            keys.dedup();
            Body::Revocation(keys)
        }
        Body::Rotation(mut entries) => {
            entries.sort_by_key(|(index, _)| *index);
            Body::Rotation(entries)
        }
        genesis => genesis,
    };
    unsigned(Core {
        network_id: previous.core.network_id,
        sequence: previous.core.sequence + 1,
        previous: previous.core.hash(),
        body,
    })
}

/// One holder signs a draft as governance key `index`.
pub fn sign_part(draft: &Statement, key_path: &Path, index: u8) -> io::Result<Part> {
    if index as usize >= super::KEY_COUNT {
        return Err(invalid("key index must be 0 to 9"));
    }
    let key = read_key(key_path)?;
    Ok(Part {
        version: 1,
        hash: hex(&draft.core.hash()),
        index,
        signature: hex(&sign(&draft.core, &key)),
    })
}

pub fn write_part(path: &Path, part: &Part) -> io::Result<()> {
    create(
        path,
        &serde_json::to_vec_pretty(part).map_err(invalid_json)?,
        0o644,
    )
}

pub fn read_part(path: &Path) -> io::Result<Part> {
    serde_json::from_slice(&fs::read(path)?).map_err(invalid_json)
}

/// The draft with the parts' signatures in index order. Parts for another
/// draft, or two parts for one index, are refused.
pub fn combine(draft: &Statement, parts: &[Part]) -> io::Result<Statement> {
    let hash = hex(&draft.core.hash());
    let mut signatures = Vec::with_capacity(parts.len());
    for part in parts {
        if part.version != 1 || part.hash != hash {
            return Err(invalid(format!(
                "part for key {} signs another statement",
                part.index
            )));
        }
        signatures.push((part.index, unhex::<64>(&part.signature)?));
    }
    signatures.sort_by_key(|(index, _)| *index);
    if signatures.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid("two parts for the same key index"));
    }
    Ok(Statement {
        core: draft.core.clone(),
        signatures,
    })
}

/// Applies `chain` (genesis first, pinned to `expect_genesis`) and returns
/// the head, exactly as a node would.
pub fn replay(chain: &[Statement], expect_genesis: &Hash) -> io::Result<Head> {
    let mut head: Option<Head> = None;
    for statement in chain {
        match apply(head.as_ref(), statement, Some(expect_genesis)).map_err(gov)? {
            Outcome::Applied(next) => head = Some(next),
            Outcome::AlreadyApplied => {}
            Outcome::Fork(_) => return Err(invalid("the chain contains a fork")),
        }
    }
    head.ok_or_else(|| invalid("empty chain"))
}

/// A readable summary of a statement.
pub fn describe(statement: &Statement) -> Value {
    let core = &statement.core;
    let b64 = |key: &Key| STANDARD.encode(key);
    let body = match &core.body {
        Body::Genesis { threshold, keys } => json!({
            "kind": "genesis",
            "threshold": threshold,
            "keys": keys.iter().map(b64).collect::<Vec<_>>(),
        }),
        Body::Rotation(entries) => json!({
            "kind": "rotation",
            "replace": entries.iter().map(|(i, k)| json!({"index": i, "key": b64(k)})).collect::<Vec<_>>(),
        }),
        Body::Revocation(keys) => json!({
            "kind": "revocation",
            "members": keys.iter().map(b64).collect::<Vec<_>>(),
        }),
    };
    json!({
        "network_id": hex(&core.network_id),
        "sequence": core.sequence,
        "previous": hex(&core.previous),
        "hash": hex(&core.hash()),
        "body": body,
        "signed_by": statement.signatures.iter().map(|(i, _)| i).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// Ten holders generate keys, draft and sign a genesis, then a
    /// revocation, entirely through files.
    #[test]
    fn a_ceremony_through_files() {
        let dir = dir();
        let keys: Vec<_> = (0..10)
            .map(|i| {
                let path = dir.path().join(format!("holder{i}.key"));
                let public = keygen(&path).unwrap();
                (path, public)
            })
            .collect();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&keys[0].0).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // keygen never replaces an existing key file.
        assert!(keygen(&keys[0].0).is_err());

        let publics: Vec<Key> = keys
            .iter()
            .map(|(_, b64)| public_key(b64).unwrap())
            .collect();
        let draft = draft_genesis(&publics, None);
        let draft_path = dir.path().join("genesis.draft");
        write_statement(&draft_path, &draft).unwrap();
        let draft = read_statement(&draft_path).unwrap();
        let parts: Vec<Part> = [0u8, 2, 3, 5, 6, 8, 9]
            .iter()
            .map(|&i| {
                let part = sign_part(&draft, &keys[i as usize].0, i).unwrap();
                let path = dir.path().join(format!("genesis.{i}.part"));
                write_part(&path, &part).unwrap();
                read_part(&path).unwrap()
            })
            .collect();
        let genesis = combine(&draft, &parts).unwrap();
        let hash = genesis.core.hash();
        let head = replay(std::slice::from_ref(&genesis), &hash).unwrap();
        assert_eq!(head.keys(), publics.as_slice());

        let member = SigningKey::generate(&mut OsRng).verifying_key().to_bytes();
        let draft = draft_after(&genesis, Body::Revocation(vec![member, member]));
        let parts: Vec<Part> = (0u8..7)
            .map(|i| sign_part(&draft, &keys[i as usize].0, i).unwrap())
            .collect();
        let revocation = combine(&draft, &parts).unwrap();
        let head = replay(&[genesis.clone(), revocation.clone()], &hash).unwrap();
        assert_eq!(head.revoked, vec![member]);
        assert_eq!(describe(&revocation)["body"]["kind"], "revocation");
        assert_eq!(describe(&revocation)["sequence"], 1);

        // A part for another draft, a duplicate index, and a statement
        // signed by too few holders are all caught.
        let other = draft_after(&genesis, Body::Revocation(vec![publics[0]]));
        let stray = sign_part(&other, &keys[7].0, 7).unwrap();
        assert!(combine(&draft, &[parts[0].clone(), stray]).is_err());
        assert!(combine(&draft, &[parts[1].clone(), parts[1].clone()]).is_err());
        let short = combine(&draft, &parts[..6]).unwrap();
        assert!(replay(&[genesis, short], &hash).is_err());
    }

    #[test]
    fn key_files_are_checked() {
        let dir = dir();
        let path = dir.path().join("holder.key");
        keygen(&path).unwrap();
        let draft = draft_genesis(&[[1; 32]; 10], Some([7; 32]));
        assert!(sign_part(&draft, &path, 10).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
            assert!(
                sign_part(&draft, &path, 0).is_err(),
                "group-readable key accepted"
            );
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(sign_part(&draft, &path, 0).is_ok());
        // A key file whose public key does not match its secret.
        let text = fs::read_to_string(&path).unwrap();
        let mut file: Value = serde_json::from_str(&text).unwrap();
        file["public_b64"] = STANDARD.encode([9u8; 32]).into();
        let tampered = dir.path().join("tampered.key");
        create(&tampered, file.to_string().as_bytes(), 0o600).unwrap();
        assert!(sign_part(&draft, &tampered, 0).is_err());
    }
}
