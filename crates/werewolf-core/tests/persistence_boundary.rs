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

// Historical failure characterization. Replace these expectations with
// rejection/preservation assertions when the safe persistence path is connected.
#[test]
fn pelt_and_pack_currently_follow_destination_symlinks() {
    let tmp = fixture();
    let victim = tmp.path().join("victim");
    std::fs::write(&victim, b"original").unwrap();
    let pelt = tmp.path().join("pelt.json");
    symlink(&victim, &pelt).unwrap();
    save_identity(&pelt, &generate_identity()).unwrap();
    assert_ne!(std::fs::read(&victim).unwrap(), b"original");
    std::fs::write(&victim, b"original").unwrap();
    let pack = tmp.path().join("pack.json");
    symlink(&victim, &pack).unwrap();
    save_pack(&pack, &[]).unwrap();
    assert_eq!(std::fs::read(&victim).unwrap(), b"[]");
}

#[test]
fn pelt_currently_overwrites_hard_linked_victim() {
    let tmp = fixture();
    let victim = tmp.path().join("victim");
    std::fs::write(&victim, b"original").unwrap();
    let pelt = tmp.path().join("pelt.json");
    std::fs::hard_link(&victim, &pelt).unwrap();
    save_identity(&pelt, &generate_identity()).unwrap();
    assert_ne!(std::fs::read(&victim).unwrap(), b"original");
}
