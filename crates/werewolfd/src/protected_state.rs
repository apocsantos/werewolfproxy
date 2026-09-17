//! Local consistency storage for the security-relevant policy group.
//!
//! Two slots hold complete candidate generations. A small regular-file pointer
//! is the only activation record: it is replaced only after every document and
//! the matching manifest are durable. This detects partial or mixed local
//! restoration relative to the current pointer. It deliberately does not make
//! a complete historical Den snapshot fresh.
use crate::target_policy::{self, TargetPolicy};
use serde::{Deserialize, Serialize};
use std::{ffi::OsStr, io};
use werewolf_core::{
    fang_profile::FangProfile,
    local_fs::{CommitOutcome, PrivateDirectory},
    pack::PeerRecord,
    state_validation,
};

const SCHEMA: u8 = 1;
const MODE_FILE: &str = "security_state_manifest_mode.json";
const CURRENT_FILE: &str = "security_state_current.json";
const MANIFEST_LIMIT: usize = 4096;
const DOCUMENT_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Slot {
    A,
    B,
}

impl Slot {
    fn inactive(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Mode {
    schema: u8,
    mode: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Current {
    schema: u8,
    generation: u64,
    slot: Slot,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DocumentCommitments {
    pack: String,
    silver: String,
    target_policy: String,
    fangs: String,
    active_fangs: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u8,
    generation: u64,
    documents: DocumentCommitments,
    state_commitment: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SilverDocument {
    version: u8,
    mode: String,
}

/// The complete set of policy values covered by one manifest generation.
#[derive(Clone)]
pub(super) struct ProtectedState {
    pub(super) silver_locked: bool,
    pub(super) peers: Vec<PeerRecord>,
    pub(super) target_policy: TargetPolicy,
    pub(super) fang_profiles: Vec<FangProfile>,
    pub(super) active_profiles: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Generation {
    pub(super) number: u64,
    slot: Slot,
}

pub(super) enum Load {
    Legacy,
    Manifest {
        generation: Generation,
        state: ProtectedState,
    },
}

pub(super) enum Transaction {
    DurablyCommitted(Generation),
    NotCommitted(io::Error),
    IndeterminateAfterRename(io::Error),
}

struct Documents {
    pack: Vec<u8>,
    silver: Vec<u8>,
    target_policy: Vec<u8>,
    fangs: Vec<u8>,
    active_fangs: Vec<u8>,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid protected state generation",
    )
}

fn name(slot: Slot, document: &str) -> String {
    format!("security_state_{}_{}.json", slot.name(), document)
}

fn canonical<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| io::Error::other("protected state serialization failed"))
}

fn exact<T>(bytes: &[u8]) -> io::Result<T>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if canonical(&value)? != bytes {
        return Err(invalid());
    }
    Ok(value)
}

fn canonical_pack(peers: &[PeerRecord]) -> io::Result<Vec<u8>> {
    state_validation::pack(peers)?;
    let mut ordered = peers.to_vec();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    canonical(&ordered)
}

fn parse_pack(bytes: &[u8]) -> io::Result<Vec<PeerRecord>> {
    let peers: Vec<PeerRecord> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if canonical_pack(&peers)? != bytes {
        return Err(invalid());
    }
    Ok(peers)
}

fn canonical_silver(locked: bool) -> io::Result<Vec<u8>> {
    canonical(&SilverDocument {
        version: 1,
        mode: if locked { "locked" } else { "open" }.into(),
    })
}

fn parse_silver(bytes: &[u8]) -> io::Result<bool> {
    let value: SilverDocument = exact(bytes)?;
    if value.version != 1 {
        return Err(invalid());
    }
    match value.mode.as_str() {
        "locked" => Ok(true),
        "open" => Ok(false),
        _ => Err(invalid()),
    }
}

fn canonical_profiles(profiles: &[FangProfile]) -> io::Result<Vec<u8>> {
    state_validation::profile_document(profiles)?;
    let mut ordered = profiles.to_vec();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    canonical(&ordered)
}

fn parse_profiles(bytes: &[u8]) -> io::Result<Vec<FangProfile>> {
    let profiles: Vec<FangProfile> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if canonical_profiles(&profiles)? != bytes {
        return Err(invalid());
    }
    Ok(profiles)
}

fn canonical_active(active: &[String], profiles: &[FangProfile]) -> io::Result<Vec<u8>> {
    state_validation::active(active, profiles)?;
    let mut ordered = active.to_vec();
    ordered.sort();
    canonical(&ordered)
}

fn parse_active(bytes: &[u8], profiles: &[FangProfile]) -> io::Result<Vec<String>> {
    let active: Vec<String> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if canonical_active(&active, profiles)? != bytes {
        return Err(invalid());
    }
    Ok(active)
}

fn documents(state: &ProtectedState) -> io::Result<Documents> {
    Ok(Documents {
        pack: canonical_pack(&state.peers)?,
        silver: canonical_silver(state.silver_locked)?,
        target_policy: target_policy::canonical_from_runtime(&state.target_policy)?,
        fangs: canonical_profiles(&state.fang_profiles)?,
        active_fangs: canonical_active(&state.active_profiles, &state.fang_profiles)?,
    })
}

fn document_commitment(document: &str, contents: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"werewolfproxy/state-manifest/v1/");
    hasher.update(document.as_bytes());
    hasher.update(b"\0schema\0");
    hasher.update(&[SCHEMA]);
    hasher.update(b"\0");
    hasher.update(contents);
    *hasher.finalize().as_bytes()
}

fn state_commitment(generation: u64, documents: &DocumentCommitments) -> io::Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"werewolfproxy/state-manifest/v1/state\0schema\0");
    hasher.update(&[SCHEMA]);
    hasher.update(b"\0generation\0");
    hasher.update(&generation.to_be_bytes());
    for (name, digest) in [
        ("pack", &documents.pack),
        ("silver", &documents.silver),
        ("target-policy", &documents.target_policy),
        ("fangs", &documents.fangs),
        ("active-fangs", &documents.active_fangs),
    ] {
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        hasher.update(&decode_digest(digest)?);
    }
    Ok(*hasher.finalize().as_bytes())
}

fn encode_digest(digest: [u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn decode_digest(value: &str) -> io::Result<[u8; 32]> {
    if value.len() != 64 {
        return Err(invalid());
    }
    let mut output = [0u8; 32];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let Some(high) = decode_nibble(pair[0]) else {
            return Err(invalid());
        };
        let Some(low) = decode_nibble(pair[1]) else {
            return Err(invalid());
        };
        output[index] = high << 4 | low;
    }
    Ok(output)
}

fn manifest(generation: u64, contents: &Documents) -> io::Result<Manifest> {
    let documents = DocumentCommitments {
        pack: encode_digest(document_commitment("pack", &contents.pack)),
        silver: encode_digest(document_commitment("silver", &contents.silver)),
        target_policy: encode_digest(document_commitment(
            "target-policy",
            &contents.target_policy,
        )),
        fangs: encode_digest(document_commitment("fangs", &contents.fangs)),
        active_fangs: encode_digest(document_commitment("active-fangs", &contents.active_fangs)),
    };
    Ok(Manifest {
        schema: SCHEMA,
        generation,
        state_commitment: encode_digest(state_commitment(generation, &documents)?),
        documents,
    })
}

fn validate_manifest(
    value: &Manifest,
    generation: Generation,
    contents: &Documents,
) -> io::Result<()> {
    if value.schema != SCHEMA || value.generation != generation.number {
        return Err(invalid());
    }
    let expected = manifest(generation.number, contents)?;
    if value.documents.pack != expected.documents.pack
        || value.documents.silver != expected.documents.silver
        || value.documents.target_policy != expected.documents.target_policy
        || value.documents.fangs != expected.documents.fangs
        || value.documents.active_fangs != expected.documents.active_fangs
        || value.state_commitment != expected.state_commitment
    {
        return Err(invalid());
    }
    Ok(())
}

fn read(den: &PrivateDirectory, file: &str, limit: usize) -> io::Result<Option<Vec<u8>>> {
    den.read(OsStr::new(file), limit)
}

fn mode_bytes() -> io::Result<Vec<u8>> {
    canonical(&Mode {
        schema: SCHEMA,
        mode: "manifest".into(),
    })
}

fn current_bytes(generation: Generation) -> io::Result<Vec<u8>> {
    canonical(&Current {
        schema: SCHEMA,
        generation: generation.number,
        slot: generation.slot,
    })
}

fn load_current(den: &PrivateDirectory) -> io::Result<Option<Generation>> {
    let mode = read(den, MODE_FILE, MANIFEST_LIMIT)?;
    let current = read(den, CURRENT_FILE, MANIFEST_LIMIT)?;
    match (mode, current) {
        (None, None) => {
            // A selector/marker deletion must never downgrade an already
            // manifest-enabled Den to the legacy flat-file loader. A complete
            // historical Den from before migration has none of these artifacts
            // and remains the documented external-anchor nonclaim.
            for slot in [Slot::A, Slot::B] {
                for document in [
                    "pack",
                    "silver",
                    "target-policy",
                    "fangs",
                    "active-fangs",
                    "manifest",
                ] {
                    if read(den, &name(slot, document), DOCUMENT_LIMIT)?.is_some() {
                        return Err(invalid());
                    }
                }
            }
            Ok(None)
        }
        (Some(mode), Some(current)) => {
            let mode: Mode = exact(&mode)?;
            let current: Current = exact(&current)?;
            if mode.schema != SCHEMA
                || mode.mode != "manifest"
                || current.schema != SCHEMA
                || current.generation == 0
            {
                return Err(invalid());
            }
            Ok(Some(Generation {
                number: current.generation,
                slot: current.slot,
            }))
        }
        _ => Err(invalid()),
    }
}

pub(super) fn load(den: &PrivateDirectory) -> io::Result<Load> {
    let Some(generation) = load_current(den)? else {
        return Ok(Load::Legacy);
    };
    let pack = read(den, &name(generation.slot, "pack"), DOCUMENT_LIMIT)?.ok_or_else(invalid)?;
    let silver =
        read(den, &name(generation.slot, "silver"), DOCUMENT_LIMIT)?.ok_or_else(invalid)?;
    let target_policy =
        read(den, &name(generation.slot, "target-policy"), DOCUMENT_LIMIT)?.ok_or_else(invalid)?;
    let fangs = read(den, &name(generation.slot, "fangs"), DOCUMENT_LIMIT)?.ok_or_else(invalid)?;
    let active_fangs =
        read(den, &name(generation.slot, "active-fangs"), DOCUMENT_LIMIT)?.ok_or_else(invalid)?;
    let manifest_bytes =
        read(den, &name(generation.slot, "manifest"), MANIFEST_LIMIT)?.ok_or_else(invalid)?;
    let manifest: Manifest = exact(&manifest_bytes)?;
    let contents = Documents {
        pack,
        silver,
        target_policy,
        fangs,
        active_fangs,
    };
    validate_manifest(&manifest, generation, &contents)?;
    let state = ProtectedState {
        silver_locked: parse_silver(&contents.silver)?,
        peers: parse_pack(&contents.pack)?,
        target_policy: target_policy::parse_canonical(&contents.target_policy)?,
        fang_profiles: parse_profiles(&contents.fangs)?,
        active_profiles: parse_active(&contents.active_fangs, &parse_profiles(&contents.fangs)?)?,
    };
    Ok(Load::Manifest { generation, state })
}

fn legacy_silver(den: &PrivateDirectory) -> io::Result<bool> {
    match read(den, "silver.json", MANIFEST_LIMIT)? {
        None => Ok(true),
        Some(bytes) => parse_silver(&canonical_silver_from_legacy(&bytes)?),
    }
}

fn canonical_silver_from_legacy(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let value: SilverDocument = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if value.version != 1 {
        return Err(invalid());
    }
    match value.mode.as_str() {
        "locked" => canonical_silver(true),
        "open" => canonical_silver(false),
        _ => Err(invalid()),
    }
}

/// Read legacy flat files into a canonical candidate for explicit migration.
/// A malformed legacy policy is intentionally not silently converted to Deny.
pub(super) fn legacy_state(den: &PrivateDirectory) -> io::Result<ProtectedState> {
    let silver_locked = legacy_silver(den)?;
    let peers = match read(den, "pack.json", DOCUMENT_LIMIT)? {
        None => Vec::new(),
        Some(bytes) => {
            let peers: Vec<PeerRecord> = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            state_validation::pack(&peers)?;
            peers
        }
    };
    let target_policy = match read(den, "target_policy.json", DOCUMENT_LIMIT)? {
        None => TargetPolicy::Deny,
        Some(bytes) => {
            let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
            let canonical = target_policy::canonicalize(text)?;
            target_policy::parse_canonical(&canonical)?
        }
    };
    let fang_profiles = match read(den, "fangs.json", DOCUMENT_LIMIT)? {
        None => Vec::new(),
        Some(bytes) => {
            let profiles: Vec<FangProfile> =
                serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            state_validation::profile_document(&profiles)?;
            profiles
        }
    };
    let active_profiles = match read(den, "active_fangs.json", DOCUMENT_LIMIT)? {
        None => Vec::new(),
        Some(bytes) => {
            let active: Vec<String> = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            state_validation::active(&active, &fang_profiles)?;
            active
        }
    };
    Ok(ProtectedState {
        silver_locked,
        peers,
        target_policy,
        fang_profiles,
        active_profiles,
    })
}

fn map_outcome(outcome: CommitOutcome) -> Result<(), Transaction> {
    match outcome {
        CommitOutcome::DurablyCommitted => Ok(()),
        CommitOutcome::NotCommitted(error) => Err(Transaction::NotCommitted(error)),
        CommitOutcome::IndeterminateAfterRename(error) => {
            Err(Transaction::IndeterminateAfterRename(error))
        }
    }
}

trait Storage {
    fn replace(&self, name: &OsStr, data: &[u8], create_only: bool) -> CommitOutcome;
}

impl Storage for PrivateDirectory {
    fn replace(&self, name: &OsStr, data: &[u8], create_only: bool) -> CommitOutcome {
        Self::replace(self, name, data, create_only)
    }
}

fn write_slot<S: Storage>(
    storage: &S,
    generation: Generation,
    state: &ProtectedState,
) -> Result<(), Transaction> {
    let contents = documents(state).map_err(Transaction::NotCommitted)?;
    let manifest = manifest(generation.number, &contents).map_err(Transaction::NotCommitted)?;
    for (logical, bytes) in [
        ("pack", &contents.pack),
        ("silver", &contents.silver),
        ("target-policy", &contents.target_policy),
        ("fangs", &contents.fangs),
        ("active-fangs", &contents.active_fangs),
    ] {
        map_outcome(storage.replace(OsStr::new(&name(generation.slot, logical)), bytes, false))?;
    }
    let manifest_bytes = canonical(&manifest).map_err(Transaction::NotCommitted)?;
    map_outcome(storage.replace(
        OsStr::new(&name(generation.slot, "manifest")),
        &manifest_bytes,
        false,
    ))
}

/// Create generation one from flat legacy files. The mode marker is written
/// immediately before the selector; a crash in that small interval fails closed
/// on the next startup instead of returning to legacy loading.
pub(super) fn migrate(den: &PrivateDirectory) -> Transaction {
    match load_current(den) {
        Ok(Some(generation)) => return Transaction::DurablyCommitted(generation),
        Err(error) => return Transaction::NotCommitted(error),
        Ok(None) => {}
    }
    let state = match legacy_state(den) {
        Ok(state) => state,
        Err(error) => return Transaction::NotCommitted(error),
    };
    let generation = Generation {
        number: 1,
        slot: Slot::A,
    };
    if let Err(outcome) = write_slot(den, generation, &state) {
        return outcome;
    }
    let mode = match mode_bytes() {
        Ok(bytes) => bytes,
        Err(error) => return Transaction::NotCommitted(error),
    };
    if let Err(outcome) = map_outcome(den.replace(OsStr::new(MODE_FILE), &mode, true)) {
        return outcome;
    }
    let current = match current_bytes(generation) {
        Ok(bytes) => bytes,
        Err(error) => return Transaction::NotCommitted(error),
    };
    match den.replace(OsStr::new(CURRENT_FILE), &current, true) {
        CommitOutcome::DurablyCommitted => Transaction::DurablyCommitted(generation),
        CommitOutcome::NotCommitted(error) => Transaction::NotCommitted(error),
        CommitOutcome::IndeterminateAfterRename(error) => {
            Transaction::IndeterminateAfterRename(error)
        }
    }
}

/// Commit a complete candidate into the inactive slot, then make it active by
/// durably replacing only the current-generation selector.
pub(super) fn commit(den: &PrivateDirectory, current: u64, state: &ProtectedState) -> Transaction {
    commit_with(den, current, state, den)
}

fn commit_with<S: Storage>(
    den: &PrivateDirectory,
    current: u64,
    state: &ProtectedState,
    storage: &S,
) -> Transaction {
    let observed = match load_current(den) {
        Ok(Some(observed)) if observed.number == current => observed,
        Ok(_) => return Transaction::NotCommitted(invalid()),
        Err(error) => return Transaction::NotCommitted(error),
    };
    let Some(number) = observed.number.checked_add(1) else {
        return Transaction::NotCommitted(io::Error::other("protected state generation exhausted"));
    };
    let next = Generation {
        number,
        slot: observed.slot.inactive(),
    };
    if let Err(outcome) = write_slot(storage, next, state) {
        return outcome;
    }
    let pointer = match current_bytes(next) {
        Ok(bytes) => bytes,
        Err(error) => return Transaction::NotCommitted(error),
    };
    match storage.replace(OsStr::new(CURRENT_FILE), &pointer, false) {
        CommitOutcome::DurablyCommitted => Transaction::DurablyCommitted(next),
        CommitOutcome::NotCommitted(error) => Transaction::NotCommitted(error),
        CommitOutcome::IndeterminateAfterRename(error) => {
            Transaction::IndeterminateAfterRename(error)
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::{cell::Cell, os::unix::fs::PermissionsExt, path::PathBuf};
    use werewolf_core::{
        local_fs::PrivateDirectory,
        pack::{PeerRecord, TrustLevel},
        pelt::generate_identity,
    };

    struct Fixture(PathBuf, PrivateDirectory);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "wwp-manifest-{}",
                crate::handshake::hex(&crate::handshake::random::<16>().unwrap())
            ));
            let den = PrivateDirectory::open(&path, true).unwrap();
            Self(path, den)
        }

        fn write(&self, name: &str, bytes: &[u8]) {
            assert!(matches!(
                self.1.replace(OsStr::new(name), bytes, false),
                CommitOutcome::DurablyCommitted
            ));
        }

        fn state(&self) -> ProtectedState {
            legacy_state(&self.1).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct FaultStorage<'a> {
        directory: &'a PrivateDirectory,
        boundary: usize,
        after_write: bool,
        writes: Cell<usize>,
    }

    impl Storage for FaultStorage<'_> {
        fn replace(&self, file: &OsStr, data: &[u8], create_only: bool) -> CommitOutcome {
            let index = self.writes.get();
            self.writes.set(index + 1);
            if index == self.boundary && !self.after_write {
                return CommitOutcome::NotCommitted(io::Error::other("injected before write"));
            }
            let outcome = self.directory.replace(file, data, create_only);
            if index == self.boundary
                && self.after_write
                && matches!(outcome, CommitOutcome::DurablyCommitted)
            {
                return CommitOutcome::IndeterminateAfterRename(io::Error::other(
                    "injected after write",
                ));
            }
            outcome
        }
    }

    #[test]
    fn migration_commits_generation_one_and_rejects_flat_tampering() {
        let fixture = Fixture::new();
        fixture.write("silver.json", br#"{"version":1,"mode":"open"}"#);
        assert!(matches!(
            migrate(&fixture.1),
            Transaction::DurablyCommitted(_)
        ));
        let Load::Manifest { generation, state } = load(&fixture.1).unwrap() else {
            panic!("migration did not enable manifest mode");
        };
        assert_eq!(generation.number, 1);
        assert!(!state.silver_locked);
        // Flat legacy files are ignored once the current selector exists.
        fixture.write("silver.json", br#"{"version":1,"mode":"locked"}"#);
        let Load::Manifest { state, .. } = load(&fixture.1).unwrap() else {
            panic!("manifest mode was downgraded");
        };
        assert!(!state.silver_locked);
    }

    #[test]
    fn active_document_tampering_and_missing_pointer_fail_closed() {
        let fixture = Fixture::new();
        fixture.write("silver.json", br#"{"version":1,"mode":"open"}"#);
        let Transaction::DurablyCommitted(generation) = migrate(&fixture.1) else {
            panic!("migration failed");
        };
        fixture.write(&name(generation.slot, "pack"), b"[{\"unexpected\":true}]");
        assert!(load(&fixture.1).is_err());

        // Restore the committed generation, then remove only its selector.
        std::fs::remove_file(fixture.0.join(CURRENT_FILE)).unwrap();
        assert!(load(&fixture.1).is_err());

        // Removing both selectors still leaves generation artifacts and must
        // not silently fall back to the legacy flat documents.
        std::fs::remove_file(fixture.0.join(MODE_FILE)).unwrap();
        assert!(load(&fixture.1).is_err());
    }

    #[test]
    fn complete_historical_generation_with_matching_pointer_is_accepted() {
        let fixture = Fixture::new();
        let first = fixture.state();
        let Transaction::DurablyCommitted(one) = migrate(&fixture.1) else {
            panic!("migration failed");
        };
        let mut second = first.clone();
        second.silver_locked = !first.silver_locked;
        let Transaction::DurablyCommitted(two) = commit(&fixture.1, one.number, &second) else {
            panic!("second generation failed");
        };
        assert_eq!(
            load(&fixture.1).unwrap().unwrap_generation().number,
            two.number
        );
        // The old slot and matching old selector remain internally valid. This
        // models the documented whole-Den historical-snapshot nonclaim.
        let old_pointer = current_bytes(one).unwrap();
        fixture.write(CURRENT_FILE, &old_pointer);
        assert_eq!(
            load(&fixture.1).unwrap().unwrap_generation().number,
            one.number
        );
    }

    #[test]
    fn every_commit_boundary_restarts_old_new_or_fail_closed_never_mixed() {
        // Commit writes five documents and a manifest before the final current
        // selector. Only the selector can activate the new complete slot.
        for after_write in [false, true] {
            for boundary in 0..7 {
                let fixture = Fixture::new();
                let first = fixture.state();
                let Transaction::DurablyCommitted(one) = migrate(&fixture.1) else {
                    panic!("migration failed");
                };
                let mut next = first.clone();
                next.silver_locked = !first.silver_locked;
                let storage = FaultStorage {
                    directory: &fixture.1,
                    boundary,
                    after_write,
                    writes: Cell::new(0),
                };
                let outcome = commit_with(&fixture.1, one.number, &next, &storage);
                if after_write {
                    assert!(matches!(outcome, Transaction::IndeterminateAfterRename(_)));
                } else {
                    assert!(matches!(outcome, Transaction::NotCommitted(_)));
                }
                match load(&fixture.1).unwrap() {
                    Load::Manifest { generation, state } if boundary == 6 && after_write => {
                        assert_eq!(generation.number, 2);
                        assert_eq!(state.silver_locked, next.silver_locked);
                    }
                    Load::Manifest { generation, state } => {
                        assert_eq!(generation.number, 1);
                        assert_eq!(state.silver_locked, first.silver_locked);
                    }
                    Load::Legacy => panic!("manifest mode was downgraded"),
                }
            }
        }
    }

    #[test]
    fn every_protected_document_rejects_valid_prior_generation_contents() {
        let fixture = Fixture::new();
        let first = fixture.state();
        let Transaction::DurablyCommitted(one) = migrate(&fixture.1) else {
            panic!("migration failed");
        };
        let identity = generate_identity();
        let mut second = first.clone();
        second.silver_locked = false;
        second.peers = vec![PeerRecord {
            name: "peer".into(),
            fingerprint: identity.fingerprint.clone(),
            address: "tcp://127.0.0.1:1".into(),
            trust: TrustLevel::Packmate,
            public_key_b64: Some(identity.public_key_b64.clone()),
        }];
        second.target_policy = target_policy::parse_canonical(
            &target_policy::canonicalize(
                &serde_json::json!({
                    "mode":"deny-by-default",
                    "peers": { identity.fingerprint.clone(): {"targets":[{"address":"127.0.0.1","port":1}]} }
                })
                .to_string(),
            )
            .unwrap(),
        )
        .unwrap();
        second.fang_profiles = vec![FangProfile {
            name: "route".into(),
            peer: "peer".into(),
            local: "127.0.0.1:1".into(),
            remote: "127.0.0.1:2".into(),
            transport: "tcp".into(),
        }];
        second.active_profiles = vec!["route".into()];
        let Transaction::DurablyCommitted(two) = commit(&fixture.1, one.number, &second) else {
            panic!("second generation failed");
        };
        for document in ["pack", "silver", "target-policy", "fangs", "active-fangs"] {
            let fixture = Fixture::new();
            // Build the same two generations independently so each corruption
            // starts from a valid current generation.
            let first = fixture.state();
            let Transaction::DurablyCommitted(one) = migrate(&fixture.1) else {
                panic!("migration failed");
            };
            let Transaction::DurablyCommitted(two) = commit(&fixture.1, one.number, &second) else {
                panic!("second generation failed");
            };
            let prior = std::fs::read(fixture.0.join(name(one.slot, document))).unwrap();
            fixture.write(&name(two.slot, document), &prior);
            assert!(load(&fixture.1).is_err(), "{document} rollback activated");
            let _ = first;
        }
        let _ = two;
    }

    trait LoadGeneration {
        fn unwrap_generation(self) -> Generation;
    }

    impl LoadGeneration for Load {
        fn unwrap_generation(self) -> Generation {
            match self {
                Load::Manifest { generation, .. } => generation,
                Load::Legacy => panic!("unexpected legacy state"),
            }
        }
    }

    #[test]
    fn manifest_schema_and_encoding_are_strict() {
        let fixture = Fixture::new();
        let Transaction::DurablyCommitted(generation) = migrate(&fixture.1) else {
            panic!("migration failed");
        };
        let manifest_name = name(generation.slot, "manifest");
        for invalid_manifest in [
            br#"{"schema":2,"generation":1,"documents":{},"state_commitment":""}"#.as_slice(),
            br#"{"schema":1,"generation":1,"documents":{"pack":"00","silver":"00","target_policy":"00","fangs":"00","active_fangs":"00","extra":"00"},"state_commitment":"00"}"#.as_slice(),
            br#"{"schema":1,"generation":1,"documents":{"pack":"00","silver":"00","target_policy":"00","fangs":"00","active_fangs":"00"},"state_commitment":"00","unknown":true}"#.as_slice(),
        ] {
            fixture.write(&manifest_name, invalid_manifest);
            assert!(load(&fixture.1).is_err());
        }
    }

    #[test]
    fn generation_overflow_fails_closed_without_wraparound() {
        let fixture = Fixture::new();
        let generation = Generation {
            number: u64::MAX,
            slot: Slot::A,
        };
        assert!(write_slot(&fixture.1, generation, &fixture.state()).is_ok());
        fixture.write(MODE_FILE, &mode_bytes().unwrap());
        fixture.write(CURRENT_FILE, &current_bytes(generation).unwrap());
        assert!(matches!(
            commit(&fixture.1, u64::MAX, &fixture.state()),
            Transaction::NotCommitted(_)
        ));
        assert_eq!(
            load(&fixture.1).unwrap().unwrap_generation().number,
            u64::MAX
        );
    }

    #[test]
    fn canonical_commitments_do_not_depend_on_hashmap_order() {
        let first = generate_identity();
        let second = generate_identity();
        let one = target_policy::canonicalize(
            &serde_json::json!({
                "mode":"deny-by-default",
                "peers": {
                    first.fingerprint.clone(): {"targets":[{"address":"127.0.0.1","port":1}]},
                    second.fingerprint.clone(): {"targets":[{"address":"127.0.0.1","port":2}]}
                }
            })
            .to_string(),
        )
        .unwrap();
        let two = target_policy::canonicalize(
            &serde_json::json!({
                "mode":"deny-by-default",
                "peers": {
                    second.fingerprint.clone(): {"targets":[{"address":"127.0.0.1","port":2}]},
                    first.fingerprint.clone(): {"targets":[{"address":"127.0.0.1","port":1}]}
                }
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(one, two);
    }

    #[test]
    fn private_directory_mode_remains_required() {
        let fixture = Fixture::new();
        std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(load(&fixture.1).is_err());
    }
}
