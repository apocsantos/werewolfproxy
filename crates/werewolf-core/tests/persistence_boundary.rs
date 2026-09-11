#![cfg(target_os = "linux")]
use std::os::unix::fs::{symlink, PermissionsExt};
use werewolf_core::{
    pack::save_pack,
    pelt::{generate_identity, save_identity},
};

fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    tmp
}

// Regression expectations for the formerly unsafe standalone persistence APIs.
#[test]
fn pelt_and_pack_reject_destination_symlinks() {
    let tmp = fixture();
    let victim = tmp.path().join("victim");
    std::fs::write(&victim, b"original").unwrap();
    let pelt = tmp.path().join("pelt.json");
    symlink(&victim, &pelt).unwrap();
    assert!(save_identity(&pelt, &generate_identity()).is_err());
    assert_eq!(std::fs::read(&victim).unwrap(), b"original");
    std::fs::write(&victim, b"original").unwrap();
    let pack = tmp.path().join("pack.json");
    symlink(&victim, &pack).unwrap();
    assert!(save_pack(&pack, &[]).is_err());
    assert_eq!(std::fs::read(&victim).unwrap(), b"original");
}

#[test]
fn pelt_rejects_hard_linked_victim() {
    let tmp = fixture();
    let victim = tmp.path().join("victim");
    std::fs::write(&victim, b"original").unwrap();
    let pelt = tmp.path().join("pelt.json");
    std::fs::hard_link(&victim, &pelt).unwrap();
    assert!(save_identity(&pelt, &generate_identity()).is_err());
    assert_eq!(std::fs::read(&victim).unwrap(), b"original");
}

#[test]
fn standalone_identity_is_validated_and_initialization_only() {
    use werewolf_core::pelt::load_identity;
    let tmp = fixture();
    let path = tmp.path().join("pelt.json");
    let identity = generate_identity();
    save_identity(&path, &identity).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o600
    );
    let before = std::fs::read(&path).unwrap();
    assert!(save_identity(&path, &generate_identity()).is_err());
    assert!(std::fs::read(&path).unwrap() == before);
    assert_eq!(
        load_identity(&path).unwrap().unwrap().fingerprint,
        identity.fingerprint
    );
    let mut invalid = identity;
    invalid.fingerprint = "invalid".into();
    std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert!(load_identity(&path).is_err());
}

#[test]
fn standalone_writer_cannot_bypass_daemon_den_lock() {
    let tmp = fixture();
    let den = werewolf_core::local_fs::PrivateDirectory::open(tmp.path(), false).unwrap();
    let _owner = den.lock(std::ffi::OsStr::new(".den.lock")).unwrap();
    assert!(save_pack(&tmp.path().join("pack.json"), &[]).is_err());
    assert!(!tmp.path().join("pack.json").exists());
}
