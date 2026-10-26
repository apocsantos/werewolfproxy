use super::*;
use ed25519_dalek::SigningKey;
use std::os::unix::fs::PermissionsExt;
use werewolf_core::governance::{apply, sign, Core, Outcome};

/// A private temporary directory, removed on drop.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Restore permissions a test may have narrowed, then remove.
        let _ = std::process::Command::new("chmod")
            .args(["-R", "u+rwx"])
            .arg(&self.0)
            .status();
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A Den in a private temporary directory, holding its lock like a daemon.
struct Den {
    dir: TempDir,
    den: PrivateDirectory,
    _lock: std::fs::File,
}

impl Den {
    fn new() -> Den {
        let path = std::env::temp_dir().join(format!(
            "wwp-terminal-{}",
            crate::handshake::hex(&crate::handshake::random::<16>().unwrap())
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let dir = TempDir(path);
        let den = PrivateDirectory::open(dir.path(), false).unwrap();
        let lock = den.lock(OsStr::new(DEN_LOCK)).unwrap();
        Den {
            dir,
            den,
            _lock: lock,
        }
    }

    fn put(&self, name: &str, bytes: &[u8]) {
        assert!(matches!(
            self.den.replace(OsStr::new(name), bytes, false),
            CommitOutcome::DurablyCommitted
        ));
    }

    /// A live-looking Den: identity, both protected slots, Den state, the
    /// governance archive, a subdirectory, and a leftover temporary file.
    fn populate(&self) {
        self.put("pelt.json", b"{\"secret\":\"PELT-SECRET\"}");
        self.put("security_state_a_pack.json", b"PACK-A");
        self.put("security_state_b_manifest.json", b"MANIFEST-B");
        self.put("security_state_current.json", b"a");
        self.put("routes.json", b"ROUTES");
        self.put("governance_00000000.wwgov", b"GENESIS");
        self.put(".state-0011.tmp", b"LEFTOVER");
        std::fs::create_dir(self.dir.path().join("fang_profiles")).unwrap();
        std::fs::write(self.dir.path().join("fang_profiles/web"), b"PROFILE").unwrap();
    }

    fn names(&self) -> Vec<String> {
        self.den
            .entries()
            .unwrap()
            .iter()
            .map(|n| n.to_string_lossy().into_owned())
            .collect()
    }

    fn tombstone(&self) -> Tombstone {
        let bytes = std::fs::read(self.dir.path().join(tombstone::FILE_NAME)).unwrap();
        Tombstone::parse(&bytes).unwrap()
    }
}

fn key(i: u8) -> SigningKey {
    SigningKey::from_bytes(&[i + 1; 32])
}

fn signed(core: Core) -> Statement {
    Statement {
        signatures: (0..7u8).map(|i| (i, sign(&core, &key(i)))).collect(),
        core,
    }
}

/// A genesis and a death after it; the head is DEAD at sequence 1.
fn dead_chain() -> (Head, Statement) {
    let genesis = signed(Core {
        network_id: [0x77; 32],
        sequence: 0,
        previous: [0; 32],
        body: Body::Genesis {
            threshold: 7,
            keys: (0..10).map(|i| key(i).verifying_key().to_bytes()).collect(),
        },
    });
    let Ok(Outcome::Applied(head)) = apply(None, &genesis, Some(&genesis.core.hash())) else {
        panic!("genesis");
    };
    let death = signed(Core {
        network_id: [0x77; 32],
        sequence: 1,
        previous: head.hash,
        body: Body::Death,
    });
    let Ok(Outcome::Applied(dead)) = apply(Some(&head), &death, None) else {
        panic!("death");
    };
    (dead, death)
}

const CLEAN: [&str; 2] = [".den.lock", "tombstone.json"];

#[test]
fn no_tombstone_is_a_normal_start_and_a_malformed_one_is_never_absent() {
    let den = Den::new();
    assert!(at_startup(&den.den).unwrap().is_none());
    den.put(tombstone::FILE_NAME, b"{\"format\":\"werewolf-tombstone\"}");
    assert!(at_startup(&den.den).is_err());
}

#[test]
fn death_destroys_everything_but_the_tombstone_and_the_inert_lock() {
    let den = Den::new();
    den.populate();
    let (_, death) = dead_chain();
    let tombstone = Tombstone::death(death, Destruction::Incomplete, &software()).unwrap();
    assert!(matches!(
        write(&den.den, &tombstone),
        CommitOutcome::DurablyCommitted
    ));
    assert_eq!(finish(&den.den, tombstone), Destruction::Complete);
    assert_eq!(den.names(), CLEAN);
    assert_eq!(den.tombstone().destruction, Destruction::Complete);
    assert_eq!(den.tombstone().kind(), Kind::Death);
    // Nothing from the live Den survives in the Tombstone.
    let text = std::fs::read_to_string(den.dir.path().join(tombstone::FILE_NAME)).unwrap();
    for needle in ["PELT-SECRET", "PACK-A", "ROUTES", "PROFILE", "LEFTOVER"] {
        assert!(!text.contains(needle), "{needle}");
    }
    // A complete, clean Den starts as a Tombstone and stays complete.
    let message = at_startup(&den.den).unwrap().unwrap();
    assert_eq!(message, "death tombstone, destruction complete");
    assert_eq!(den.names(), CLEAN);
}

#[test]
fn a_stray_entry_next_to_a_complete_tombstone_downgrades_it_first() {
    // I3.
    let den = Den::new();
    den.put(
        tombstone::FILE_NAME,
        &Tombstone::retirement(Destruction::Complete, &software())
            .unwrap()
            .encode()
            .unwrap(),
    );
    den.put("pelt.json", b"PELT-SECRET");
    let message = at_startup(&den.den).unwrap().unwrap();
    assert_eq!(message, "retirement tombstone, destruction complete");
    assert_eq!(den.names(), CLEAN);
    // A junk lock is scrubbed in place, never unlinked.
    std::fs::write(den.dir.path().join(DEN_LOCK), b"junk").unwrap();
    at_startup(&den.den).unwrap();
    assert!(den.den.lock_is_inert(OsStr::new(DEN_LOCK)).unwrap());
    assert_eq!(den.names(), CLEAN);
}

#[test]
fn an_undeletable_entry_never_yields_complete() {
    // I3: a subdirectory whose contents cannot be removed.
    let den = Den::new();
    den.populate();
    let stuck = den.dir.path().join("stuck");
    std::fs::create_dir(&stuck).unwrap();
    std::fs::write(stuck.join("file"), b"x").unwrap();
    std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o500)).unwrap();
    let tombstone = Tombstone::retirement(Destruction::Incomplete, &software()).unwrap();
    assert!(matches!(
        write(&den.den, &tombstone),
        CommitOutcome::DurablyCommitted
    ));
    assert_eq!(finish(&den.den, tombstone), Destruction::Incomplete);
    assert_eq!(den.tombstone().destruction, Destruction::Incomplete);
    // The Pelt went first, whatever failed later.
    assert!(!den.names().contains(&"pelt.json".to_string()));
    // Once the storage allows it, the next start completes.
    std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        at_startup(&den.den).unwrap().unwrap(),
        "retirement tombstone, destruction complete"
    );
    assert_eq!(den.names(), CLEAN);
}

/// Crash convergence: every step boundary of section 4, entered as the
/// state a crash at that point leaves on disk, converges at the next start.
#[test]
fn every_crash_point_converges_at_the_next_start() {
    let (head, death) = dead_chain();
    let archive = crate::discovery::governance_archive_name(1);
    let fresh = || {
        let den = Den::new();
        den.populate();
        den.put(&archive, &death.encode());
        den
    };
    let incomplete = Tombstone::death(death.clone(), Destruction::Incomplete, &software()).unwrap();

    // After the DEAD commit, before the Tombstone: section 6, step 2.
    let den = fresh();
    let message = dead_head_at_startup(&den.den, &head).unwrap();
    assert_eq!(message, "death tombstone, destruction complete");
    assert_eq!(den.names(), CLEAN);

    // After the incomplete Tombstone, before any destruction.
    let den = fresh();
    den.put(tombstone::FILE_NAME, &incomplete.encode().unwrap());
    assert_eq!(
        at_startup(&den.den).unwrap().unwrap(),
        "death tombstone, destruction complete"
    );
    assert_eq!(den.names(), CLEAN);

    // After the Pelt, after the security state, and before the complete
    // rewrite: each later state is a subset of the one before.
    for removed in [
        vec!["pelt.json"],
        vec![
            "pelt.json",
            "security_state_a_pack.json",
            "security_state_b_manifest.json",
            "security_state_current.json",
        ],
    ] {
        let den = fresh();
        den.put(tombstone::FILE_NAME, &incomplete.encode().unwrap());
        for name in removed {
            den.den.destroy(OsStr::new(name)).unwrap();
        }
        at_startup(&den.den).unwrap();
        assert_eq!(den.names(), CLEAN);
        assert_eq!(den.tombstone().destruction, Destruction::Complete);
    }
    let den = Den::new();
    den.put(tombstone::FILE_NAME, &incomplete.encode().unwrap());
    at_startup(&den.den).unwrap();
    assert_eq!(den.tombstone().destruction, Destruction::Complete);
}

#[test]
fn no_destruction_begins_without_a_durable_tombstone() {
    let (head, death) = dead_chain();
    let den = Den::new();
    den.populate();
    den.put(
        &crate::discovery::governance_archive_name(1),
        &death.encode(),
    );
    // The Tombstone cannot be written: a directory occupies its name.
    std::fs::create_dir(den.dir.path().join(tombstone::FILE_NAME)).unwrap();
    let message = dead_head_at_startup(&den.den, &head).unwrap();
    assert!(message.contains("not yet durable"), "{message}");
    assert!(den.names().contains(&"pelt.json".to_string()));
    assert!(den
        .names()
        .contains(&"security_state_a_pack.json".to_string()));
}

#[test]
fn a_dead_head_needs_its_archived_death_statement() {
    let (head, death) = dead_chain();
    let den = Den::new();
    den.populate();
    assert!(dead_head_at_startup(&den.den, &head).is_err());
    // Another statement under the death's archive name is refused too.
    den.put(
        &crate::discovery::governance_archive_name(1),
        &signed(Core {
            body: Body::Activation,
            ..death.core.clone()
        })
        .encode(),
    );
    assert!(dead_head_at_startup(&den.den, &head).is_err());
    assert!(den.names().contains(&"pelt.json".to_string()));
}

#[test]
fn lifecycle_reports_chain_and_node() {
    let mut live = DaemonState::default();
    assert_eq!(lifecycle(&live), json!({"chain": "none", "node": "normal"}));
    let (dead, _) = dead_chain();
    live.governance = Some(Head {
        dead_at: None,
        sequence: 0,
        ..dead.clone()
    });
    assert_eq!(lifecycle(&live)["chain"], "ceremony");
    live.governance = Some(Head {
        activated_at: Some(0),
        dead_at: None,
        ..dead.clone()
    });
    assert_eq!(lifecycle(&live)["chain"], "active");
    live.governance = Some(dead);
    live.dying = true;
    assert_eq!(lifecycle(&live), json!({"chain": "dead", "node": "dying"}));
}
