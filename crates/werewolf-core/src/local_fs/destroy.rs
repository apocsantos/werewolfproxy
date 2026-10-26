//! Stage30 destruction primitives (`docs/STAGE30_LIFECYCLE_SPEC.md`
//! section 4). Every operation is relative to the Den's pinned descriptor and
//! never follows a symlink.
//!
//! I2: whether a file is overwritten is decided from `fstat` on the
//! descriptor actually opened, never from the pre-open `statat`, which may
//! only exclude. A file swapped between the two is judged by what was opened.

use super::PrivateDirectory;
use rustix::fs::{self, AtFlags, Dir, FileType, Mode, OFlags};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Write},
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    os::unix::ffi::OsStrExt,
};

static ZEROS: [u8; 64 * 1024] = [0; 64 * 1024];

/// The names in the directory behind `fd`, without `.` and `..`.
fn names(fd: BorrowedFd<'_>) -> io::Result<Vec<OsString>> {
    let dup = fs::openat(
        fd,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let mut out = Vec::new();
    for entry in Dir::read_from(&dup)? {
        let name = entry?.file_name().to_bytes().to_vec();
        if name != b"." && name != b".." {
            out.push(OsStr::from_bytes(&name).to_os_string());
        }
    }
    out.sort();
    Ok(out)
}

/// Overwrites the regular file just opened as `fd` with zeros, if and only if
/// its own `fstat` shows one link and the expected owner (I2, L3).
fn overwrite_if_eligible(fd: OwnedFd, uid: u32) -> io::Result<()> {
    let stat = fs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_nlink != 1
        || stat.st_uid != uid
    {
        return Ok(());
    }
    let mut file = File::from(fd);
    let mut left = stat.st_size.max(0) as u64;
    while left > 0 {
        let chunk = left.min(ZEROS.len() as u64) as usize;
        file.write_all(&ZEROS[..chunk])?;
        left -= chunk as u64;
    }
    file.flush()?;
    fs::fsync(&file)?;
    Ok(())
}

/// Destroys one entry of the directory behind `dir` (spec section 4, step
/// 4). `between` runs after the pre-filter and before the open; tests use it
/// to swap the entry.
fn destroy_in(
    dir: BorrowedFd<'_>,
    name: &OsStr,
    uid: u32,
    between: &mut dyn FnMut(&OsStr),
) -> io::Result<()> {
    let stat = match fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::Directory => {
            // Contents first, deepest first, then the empty directory.
            let sub = fs::openat(
                dir,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            for child in names(sub.as_fd())? {
                destroy_in(sub.as_fd(), &child, uid, between)?;
            }
            fs::fsync(&sub)?;
            drop(sub);
            match fs::unlinkat(dir, name, AtFlags::REMOVEDIR) {
                Ok(()) | Err(rustix::io::Errno::NOENT) => Ok(()),
                Err(e) => Err(e.into()),
            }
        }
        FileType::RegularFile => {
            between(name);
            // A failed open makes the entry ineligible: unlinked only.
            if let Ok(fd) = fs::openat(
                dir,
                name,
                OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                overwrite_if_eligible(fd, uid)?;
            }
            unlink(dir, name)
        }
        // Symlinks, sockets, FIFOs and devices are never opened.
        _ => unlink(dir, name),
    }
}

fn unlink(dir: BorrowedFd<'_>, name: &OsStr) -> io::Result<()> {
    match fs::unlinkat(dir, name, AtFlags::empty()) {
        Ok(()) | Err(rustix::io::Errno::NOENT) => Ok(()),
        // A directory swapped in after the pre-filter.
        Err(rustix::io::Errno::ISDIR) => Err(io::Error::other("entry changed during destruction")),
        Err(e) => Err(e.into()),
    }
}

impl PrivateDirectory {
    /// The names in this directory, sorted, without `.` and `..`.
    pub fn entries(&self) -> io::Result<Vec<OsString>> {
        self.revalidate()?;
        names(self.fd.as_fd())
    }

    /// Destroys one entry: a regular file is overwritten once with zeros if
    /// its opened descriptor shows one link and this process's owner, then
    /// unlinked; anything else is unlinked unopened; a directory is emptied
    /// the same way and removed. A missing entry is already destroyed.
    pub fn destroy(&self, name: &OsStr) -> io::Result<()> {
        self.destroy_with(name, &mut |_| {})
    }

    pub(crate) fn destroy_with(
        &self,
        name: &OsStr,
        between: &mut dyn FnMut(&OsStr),
    ) -> io::Result<()> {
        Self::name(name)?;
        self.revalidate()?;
        destroy_in(self.fd.as_fd(), name, self.uid, between)
    }

    /// Makes every completed unlink durable.
    pub fn sync(&self) -> io::Result<()> {
        fs::fsync(&self.fd).map_err(Into::into)
    }

    /// Whether `name` is an inert lock file: a regular, empty, mode 0600 file
    /// owned by this process's owner, with one link. A missing file is inert.
    pub fn lock_is_inert(&self, name: &OsStr) -> io::Result<bool> {
        Self::name(name)?;
        self.revalidate()?;
        let stat = match fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) => return Ok(true),
            Err(e) => return Err(e.into()),
        };
        Ok(
            FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile
                && stat.st_size == 0
                && stat.st_mode & 0o7777 == 0o600
                && stat.st_uid == self.uid
                && stat.st_gid == self.gid
                && stat.st_nlink == 1,
        )
    }

    /// Empties a lock file in place, keeping its inode (and so the lock
    /// domain): overwrite with zeros, truncate, sync. Only a regular file with
    /// one link and this owner is touched; anything else is an error.
    pub fn scrub_lock(&self, name: &OsStr) -> io::Result<()> {
        Self::name(name)?;
        self.revalidate()?;
        let fd = fs::openat(
            &self.fd,
            name,
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let stat = fs::fstat(&fd)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_nlink != 1
            || stat.st_uid != self.uid
        {
            return Err(io::Error::other("lock file is not a private regular file"));
        }
        let fd = {
            let dup = fd.try_clone()?;
            overwrite_if_eligible(dup, self.uid)?;
            fd
        };
        fs::ftruncate(&fd, 0)?;
        fs::fchmod(&fd, Mode::from_raw_mode(0o600))?;
        fs::fsync(&fd)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn fixture() -> (tempfile::TempDir, PrivateDirectory) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let dir = PrivateDirectory::open(tmp.path(), false).unwrap();
        (tmp, dir)
    }

    #[test]
    fn files_and_directories_are_destroyed() {
        let (tmp, dir) = fixture();
        std::fs::write(tmp.path().join("pelt.json"), b"secret").unwrap();
        std::fs::create_dir_all(tmp.path().join("sub/deeper")).unwrap();
        std::fs::write(tmp.path().join("sub/deeper/x"), b"x").unwrap();
        for name in dir.entries().unwrap() {
            dir.destroy(&name).unwrap();
        }
        dir.sync().unwrap();
        assert!(dir.entries().unwrap().is_empty());
        // Destroying a missing entry is done already.
        dir.destroy(OsStr::new("pelt.json")).unwrap();
    }

    #[test]
    fn a_symlink_and_a_hard_link_are_unlinked_and_the_outside_is_untouched() {
        let (tmp, dir) = fixture();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("precious");
        std::fs::write(&target, b"keep me").unwrap();
        symlink(&target, tmp.path().join("link")).unwrap();
        // A hard link only works within one filesystem; tempdirs share one.
        std::fs::hard_link(&target, tmp.path().join("hard")).unwrap();
        dir.destroy(OsStr::new("link")).unwrap();
        dir.destroy(OsStr::new("hard")).unwrap();
        assert!(dir.entries().unwrap().is_empty());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep me");
    }

    #[test]
    fn eligibility_comes_from_the_opened_descriptor() {
        // I2: a regular file replaced by a symlink between the pre-filter and
        // the open is never followed or overwritten.
        let (tmp, dir) = fixture();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("precious");
        std::fs::write(&target, b"keep me").unwrap();
        std::fs::write(tmp.path().join("state.json"), b"state").unwrap();
        let path = tmp.path().to_path_buf();
        let swap_target = target.clone();
        dir.destroy_with(OsStr::new("state.json"), &mut |name| {
            std::fs::remove_file(path.join(name)).unwrap();
            symlink(&swap_target, path.join(name)).unwrap();
        })
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"keep me");
        assert!(dir.entries().unwrap().is_empty());

        // ...and one replaced by a hard link to an outside file is opened, but
        // its descriptor shows two links: not overwritten, only unlinked.
        std::fs::write(tmp.path().join("state.json"), b"state").unwrap();
        let path = tmp.path().to_path_buf();
        let swap_target = target.clone();
        dir.destroy_with(OsStr::new("state.json"), &mut |name| {
            std::fs::remove_file(path.join(name)).unwrap();
            std::fs::hard_link(&swap_target, path.join(name)).unwrap();
        })
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"keep me");
        assert!(dir.entries().unwrap().is_empty());
    }

    #[test]
    fn a_fifo_is_never_opened() {
        let (tmp, dir) = fixture();
        let path = std::ffi::CString::new(tmp.path().join("pipe").as_os_str().as_bytes()).unwrap();
        // Opening a FIFO for writing with no reader would block or fail;
        // destruction must unlink it without opening it.
        assert_eq!(unsafe { libc_mkfifo(path.as_ptr()) }, 0);
        dir.destroy(OsStr::new("pipe")).unwrap();
        assert!(dir.entries().unwrap().is_empty());
    }

    unsafe fn libc_mkfifo(path: *const std::os::raw::c_char) -> i32 {
        extern "C" {
            fn mkfifo(path: *const std::os::raw::c_char, mode: u32) -> i32;
        }
        mkfifo(path, 0o600)
    }

    #[test]
    fn the_overwrite_reaches_the_file_before_unlink() {
        // Observe the zeros through a second descriptor opened beforehand:
        // it shares the inode, so after destruction it reads the overwrite.
        let (tmp, dir) = fixture();
        std::fs::write(tmp.path().join("pelt.json"), b"secret!!").unwrap();
        let mut witness = std::fs::File::open(tmp.path().join("pelt.json")).unwrap();
        dir.destroy(OsStr::new("pelt.json")).unwrap();
        let mut seen = Vec::new();
        std::io::Read::read_to_end(&mut witness, &mut seen).unwrap();
        assert_eq!(seen, vec![0u8; 8]);
    }

    #[test]
    fn locks_are_checked_and_scrubbed_in_place() {
        let (tmp, dir) = fixture();
        let lock = OsStr::new(".den.lock");
        assert!(dir.lock_is_inert(lock).unwrap());
        let held = dir.lock(lock).unwrap();
        assert!(dir.lock_is_inert(lock).unwrap());
        std::fs::write(tmp.path().join(".den.lock"), b"junk").unwrap();
        assert!(!dir.lock_is_inert(lock).unwrap());
        use std::os::unix::fs::MetadataExt;
        let inode = std::fs::metadata(tmp.path().join(".den.lock"))
            .unwrap()
            .ino();
        dir.scrub_lock(lock).unwrap();
        assert!(dir.lock_is_inert(lock).unwrap());
        assert_eq!(
            std::fs::metadata(tmp.path().join(".den.lock"))
                .unwrap()
                .ino(),
            inode,
            "the lock inode must survive"
        );
        drop(held);
    }
}
