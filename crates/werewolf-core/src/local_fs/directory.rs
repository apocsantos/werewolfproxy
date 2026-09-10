use rustix::fs::{self, AtFlags, FileType, FlockOperation, Mode, OFlags, Stat};
use std::{
    ffi::OsStr,
    fs::File,
    io,
    os::fd::{AsRawFd, OwnedFd},
    path::{Component, Path, PathBuf},
};

pub fn effective_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "unsafe local filesystem object",
    )
}

fn credentials() -> io::Result<(u32, u32)> {
    let uid = rustix::process::geteuid();
    let gid = rustix::process::getegid();
    if uid != rustix::process::getuid() || gid != rustix::process::getgid() {
        return Err(invalid());
    }
    Ok((uid.as_raw(), gid.as_raw()))
}

fn ancestor(s: &Stat, uid: u32, gid: u32) -> io::Result<()> {
    if FileType::from_raw_mode(s.st_mode) != FileType::Directory {
        return Err(invalid());
    }
    let trusted_owner = s.st_uid == 0 || (s.st_uid == uid && s.st_gid == gid);
    // A root-owned sticky temporary ancestor is allowed, but never as the leaf.
    let sticky_root = s.st_uid == 0 && s.st_gid == 0 && s.st_mode & 0o1000 != 0;
    if !trusted_owner || (s.st_mode & 0o022 != 0 && !sticky_root) {
        return Err(invalid());
    }
    Ok(())
}

fn private(s: &Stat, uid: u32, gid: u32) -> io::Result<()> {
    if FileType::from_raw_mode(s.st_mode) != FileType::Directory
        || s.st_uid != uid
        || s.st_gid != gid
        || s.st_mode & 0o7777 != 0o700
    {
        return Err(invalid());
    }
    Ok(())
}

/// A validated private leaf pinned by an owned descriptor. Operations accept a
/// single filename; callers cannot escape through a relative path or symlink.
pub struct PrivateDirectory {
    fd: OwnedFd,
    uid: u32,
    gid: u32,
}

impl PrivateDirectory {
    pub fn open(path: &Path, create_leaf: bool) -> io::Result<Self> {
        let (uid, gid) = credentials()?;
        if !path.is_absolute() || path == Path::new("/") {
            return Err(invalid());
        }
        let parts: Vec<_> = path.components().collect();
        if parts
            .iter()
            .skip(1)
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(invalid());
        }
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut fd = fs::open("/", flags, Mode::empty())?;
        ancestor(&fs::fstat(&fd)?, uid, gid)?;
        for (index, part) in parts.iter().copied().enumerate().skip(1) {
            let Component::Normal(name) = part else {
                return Err(invalid());
            };
            let leaf = index + 1 == parts.len();
            let next = match fs::openat(&fd, name, flags, Mode::empty()) {
                Ok(next) => next,
                Err(rustix::io::Errno::NOENT) if leaf && create_leaf => {
                    fs::mkdirat(&fd, name, Mode::from_raw_mode(0o700))?;
                    fs::openat(&fd, name, flags, Mode::empty())?
                }
                Err(e) => return Err(e.into()),
            };
            let stat = fs::fstat(&next)?;
            if leaf {
                private(&stat, uid, gid)?;
            } else {
                ancestor(&stat, uid, gid)?;
            }
            fd = next;
        }
        Ok(Self { fd, uid, gid })
    }

    fn name(name: &OsStr) -> io::Result<()> {
        let mut parts = Path::new(name).components();
        if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() {
            return Err(invalid());
        }
        Ok(())
    }

    fn revalidate(&self) -> io::Result<()> {
        private(&fs::fstat(&self.fd)?, self.uid, self.gid)
    }

    /// The lock inode is retained for the lifetime of the returned file. Never
    /// unlink lock files: replacing their inode would create a second lock domain.
    pub fn lock(&self, name: &OsStr) -> io::Result<File> {
        Self::name(name)?;
        self.revalidate()?;
        let fd = fs::openat(
            &self.fd,
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::from_raw_mode(0o600),
        )?;
        let stat = fs::fstat(&fd)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_uid != self.uid
            || stat.st_gid != self.gid
            || stat.st_mode & 0o7777 != 0o600
            || stat.st_nlink != 1
        {
            return Err(invalid());
        }
        fs::flock(&fd, FlockOperation::NonBlockingLockExclusive)?;
        Ok(File::from(fd))
    }

    fn socket_stat(&self, name: &OsStr) -> io::Result<Option<Stat>> {
        Self::name(name)?;
        self.revalidate()?;
        let stat = match fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        self.validate_socket(&stat)?;
        Ok(Some(stat))
    }

    fn validate_socket(&self, stat: &Stat) -> io::Result<()> {
        if FileType::from_raw_mode(stat.st_mode) != FileType::Socket
            || stat.st_uid != self.uid
            || stat.st_gid != self.gid
            || stat.st_nlink != 1
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn socket_inode(&self, name: &OsStr) -> io::Result<Option<(u64, u64)>> {
        Ok(self.socket_stat(name)?.map(|s| (s.st_dev, s.st_ino)))
    }

    /// Linux /proc/self/fd resolves the pinned directory, including after a
    /// rename of its original pathname. This is used only for Unix socket APIs
    /// that do not expose a bindat/connectat interface.
    pub fn socket_path(&self, name: &OsStr) -> io::Result<PathBuf> {
        Self::name(name)?;
        self.revalidate()?;
        Ok(PathBuf::from(format!("/proc/self/fd/{}", self.fd.as_raw_fd())).join(name))
    }

    /// Caller must hold the instance lock and have independently established
    /// ECONNREFUSED. Missing, changed, active or ambiguous objects are preserved.
    pub fn unlink_stale_socket(&self, name: &OsStr, inode: (u64, u64)) -> io::Result<()> {
        if self.socket_inode(name)? != Some(inode) {
            return Err(invalid());
        }
        fs::unlinkat(&self.fd, name, AtFlags::empty())?;
        Ok(())
    }

    /// Restrict a newly bound socket BEFORE handing the listener to an accept
    /// loop. This is creation finalization, never repair of an existing socket.
    /// Linux rustix chmodat cannot use SYMLINK_NOFOLLOW here. Its supported
    /// pathname operation is confined to the pinned, owner/GID-validated 0700
    /// directory: other UIDs cannot substitute entries. Same-UID processes have
    /// ADMIN authority; this does not protect against malicious same-UID/root
    /// mutation. Both metadata checks use SYMLINK_NOFOLLOW, and the final check
    /// requires identical device/inode, socket type, owner, group and mode 0600.
    pub fn restrict_socket(&self, name: &OsStr) -> io::Result<()> {
        self.restrict_socket_checked(name, || {})
    }

    fn restrict_socket_checked(&self, name: &OsStr, after_chmod: impl FnOnce()) -> io::Result<()> {
        let before = self.socket_inode(name)?.ok_or_else(invalid)?;
        fs::chmodat(&self.fd, name, Mode::from_raw_mode(0o600), AtFlags::empty())?;
        after_chmod();
        let after = self.socket_stat(name)?.ok_or_else(invalid)?;
        if (after.st_dev, after.st_ino) != before || after.st_mode & 0o7777 != 0o600 {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        // Test setup only; production validation never repairs a directory.
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            std::fs::metadata(tmp.path()).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        tmp
    }

    #[test]
    fn private_directory_and_exclusive_lifetime_lock() {
        let tmp = fixture();
        let path = tmp.path().join("runtime");
        let dir = PrivateDirectory::open(&path, true).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let lock = dir.lock(OsStr::new("instance.lock")).unwrap();
        assert!(dir.lock(OsStr::new("instance.lock")).is_err());
        drop(lock);
        assert!(dir.lock(OsStr::new("instance.lock")).is_ok());
        assert!(dir.lock(OsStr::new("../escape")).is_err());
    }

    #[test]
    fn rejects_unsafe_leaf_and_symlink_traversal() {
        let tmp = fixture();
        let path = tmp.path().join("runtime");
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(PrivateDirectory::open(&path, false).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&path, tmp.path().join("link")).unwrap();
        assert!(PrivateDirectory::open(&tmp.path().join("link"), false).is_err());
        let dir = PrivateDirectory::open(&path, false).unwrap();
        std::fs::write(path.join("victim"), b"preserve").unwrap();
        assert!(dir.socket_inode(OsStr::new("victim")).is_err());
        symlink(path.join("victim"), path.join("socket")).unwrap();
        assert!(dir.socket_inode(OsStr::new("socket")).is_err());
        assert_eq!(std::fs::read(path.join("victim")).unwrap(), b"preserve");
    }
    #[test]
    fn exact_private_mode_matrix() {
        let tmp = fixture();
        let path = tmp.path().join("runtime");
        std::fs::create_dir(&path).unwrap();
        for mode in [0o700, 0o750, 0o755, 0o770, 0o775, 0o777] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                PrivateDirectory::open(&path, false).is_ok(),
                mode == 0o700,
                "mode {mode:o}"
            );
        }
    }

    #[test]
    fn newly_bound_socket_restricted_and_verified() {
        let tmp = fixture();
        let dir = PrivateDirectory::open(tmp.path(), false).unwrap();
        let name = OsStr::new("control.sock");
        let listener =
            std::os::unix::net::UnixListener::bind(dir.socket_path(name).unwrap()).unwrap();
        let before = dir.socket_inode(name).unwrap().unwrap();
        dir.restrict_socket(name).unwrap();
        assert_eq!(dir.socket_inode(name).unwrap(), Some(before));
        let stat = dir.socket_stat(name).unwrap().unwrap();
        assert_eq!(stat.st_mode & 0o7777, 0o600);
        assert_eq!((stat.st_uid, stat.st_gid), (dir.uid, dir.gid));
        // No accept call is made until successful finalization.
        drop(listener);
    }

    #[test]
    fn socket_restriction_rejects_collisions_and_unsafe_parent() {
        let tmp = fixture();
        let dir = PrivateDirectory::open(tmp.path(), false).unwrap();
        let victim = tmp.path().join("victim");
        std::fs::write(&victim, b"unchanged").unwrap();
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&victim, tmp.path().join("link")).unwrap();
        std::fs::create_dir(tmp.path().join("directory")).unwrap();
        fs::mknodat(
            &dir.fd,
            "fifo",
            FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        for name in ["victim", "link", "directory", "fifo"] {
            assert!(dir.restrict_socket(OsStr::new(name)).is_err(), "{name}");
        }
        assert_eq!(std::fs::read(&victim).unwrap(), b"unchanged");
        assert_eq!(
            std::fs::metadata(&victim).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        let name = OsStr::new("socket");
        let _listener =
            std::os::unix::net::UnixListener::bind(dir.socket_path(name).unwrap()).unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(dir.restrict_socket(name).is_err());
    }

    #[test]
    fn socket_metadata_rejects_foreign_owner_group_and_device() {
        let tmp = fixture();
        let dir = PrivateDirectory::open(tmp.path(), false).unwrap();
        let name = OsStr::new("socket");
        let _listener =
            std::os::unix::net::UnixListener::bind(dir.socket_path(name).unwrap()).unwrap();
        let original = dir.socket_stat(name).unwrap().unwrap();
        let mut stat = original;
        stat.st_uid = dir.uid.wrapping_add(1);
        assert!(dir.validate_socket(&stat).is_err());
        stat = original;
        stat.st_gid = dir.gid.wrapping_add(1);
        assert!(dir.validate_socket(&stat).is_err());
        stat = original;
        stat.st_mode = FileType::CharacterDevice.as_raw_mode() | 0o600;
        assert!(dir.validate_socket(&stat).is_err());
        stat = original;
        stat.st_nlink = 2;
        assert!(dir.validate_socket(&stat).is_err());
    }

    #[test]
    fn replacement_after_chmod_is_detected_without_unsafe_race() {
        let tmp = fixture();
        let dir = PrivateDirectory::open(tmp.path(), false).unwrap();
        let name = OsStr::new("socket");
        let path = dir.socket_path(name).unwrap();
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let replacement = dir.socket_path(OsStr::new("replacement")).unwrap();
        let _replacement_listener = std::os::unix::net::UnixListener::bind(&replacement).unwrap();
        let result = dir.restrict_socket_checked(name, || {
            std::fs::rename(&replacement, &path).unwrap();
        });
        assert!(result.is_err());
    }
}
