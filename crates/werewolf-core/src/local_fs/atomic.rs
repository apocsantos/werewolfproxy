use super::PrivateDirectory;
use rand_core::{OsRng, RngCore};
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags, RenameFlags, Stat};
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, Read, Write},
};

/// A directory fsync error AFTER rename cannot be reported as a rolled-back
/// write. Callers must enter a fatal/degraded storage state on that outcome.
#[derive(Debug)]
pub enum CommitOutcome {
    NotCommitted(io::Error),
    DurablyCommitted,
    IndeterminateAfterRename(io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    BeforeCreate,
    AfterCreate,
    MidWrite,
    BeforeFileSync,
    BeforeRename,
    AfterRename,
    AfterDirectorySync,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "unsafe persistent state")
}

impl PrivateDirectory {
    fn regular(&self, stat: &Stat) -> io::Result<()> {
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_uid != self.uid
            || stat.st_gid != self.gid
            || stat.st_mode & 0o7777 != 0o600
            || stat.st_nlink != 1
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn named_regular(&self, name: &OsStr) -> io::Result<Option<(u64, u64)>> {
        Self::name(name)?;
        self.revalidate()?;
        let stat = match fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        self.regular(&stat)?;
        Ok(Some((stat.st_dev, stat.st_ino)))
    }

    /// Read at most limit bytes from the regular file actually opened. Metadata
    /// preflight avoids opening known special devices; fstat repeats validation.
    pub fn read(&self, name: &OsStr, limit: usize) -> io::Result<Option<Vec<u8>>> {
        let Some(expected) = self.named_regular(name)? else {
            return Ok(None);
        };
        let fd = fs::openat(
            &self.fd,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )?;
        let stat = fs::fstat(&fd)?;
        self.regular(&stat)?;
        if (stat.st_dev, stat.st_ino) != expected
            || stat.st_size < 0
            || stat.st_size as u64 > limit as u64
        {
            return Err(invalid());
        }
        let mut data = Vec::new();
        File::from(fd)
            .take((limit as u64).saturating_add(1))
            .read_to_end(&mut data)?;
        if data.len() > limit {
            return Err(invalid());
        }
        Ok(Some(data))
    }

    /// The caller serializes and validates candidate state before invoking this
    /// operation, and publishes memory only on DurablyCommitted. No secret bytes
    /// appear in errors. Temporary contents are never accepted as named state.
    pub fn replace(&self, name: &OsStr, data: &[u8], create_only: bool) -> CommitOutcome {
        let mut entropy = [0u8; 16];
        if OsRng.try_fill_bytes(&mut entropy).is_err() {
            return CommitOutcome::NotCommitted(io::Error::other("persistence RNG unavailable"));
        }
        let suffix: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
        self.replace_with(
            name,
            data,
            create_only,
            OsStr::new(&format!(".state-{suffix}.tmp")),
            |_| Ok(()),
        )
    }

    fn replace_with(
        &self,
        name: &OsStr,
        data: &[u8],
        create_only: bool,
        temporary: &OsStr,
        mut point: impl FnMut(Point) -> io::Result<()>,
    ) -> CommitOutcome {
        let mut renamed = false;
        let mut created_inode = None;
        let result = (|| -> io::Result<()> {
            Self::name(temporary)?;
            if data.len() > 1024 * 1024 || name == temporary {
                return Err(invalid());
            }
            let before = self.named_regular(name)?;
            if create_only && before.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "state already initialized",
                ));
            }
            point(Point::BeforeCreate)?;
            let fd = fs::openat(
                &self.fd,
                temporary,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )?;
            let stat = fs::fstat(&fd)?;
            self.regular(&stat)?;
            created_inode = Some((stat.st_dev, stat.st_ino));
            let mut file = File::from(fd);
            point(Point::AfterCreate)?;
            let middle = data.len() / 2;
            file.write_all(&data[..middle])?;
            point(Point::MidWrite)?;
            file.write_all(&data[middle..])?;
            file.flush()?;
            point(Point::BeforeFileSync)?;
            fs::fsync(&file)?;
            point(Point::BeforeRename)?;
            if self.named_regular(name)? != before
                || self.named_regular(temporary)? != created_inode
            {
                return Err(invalid());
            }
            if before.is_none() {
                fs::renameat_with(&self.fd, temporary, &self.fd, name, RenameFlags::NOREPLACE)?;
            } else {
                fs::renameat(&self.fd, temporary, &self.fd, name)?;
            }
            renamed = true;
            point(Point::AfterRename)?;
            fs::fsync(&self.fd)?;
            point(Point::AfterDirectorySync)?;
            Ok(())
        })();
        match result {
            Ok(()) => CommitOutcome::DurablyCommitted,
            Err(error) if renamed => CommitOutcome::IndeterminateAfterRename(error),
            Err(error) => {
                // Never remove a collision or a substituted object. Cleanup
                // failures may leave a restrictive, ignored temporary document.
                if let Some(inode) = created_inode {
                    if self.named_regular(temporary).ok().flatten() == Some(inode) {
                        let _ = fs::unlinkat(&self.fd, temporary, AtFlags::empty());
                    }
                }
                CommitOutcome::NotCommitted(error)
            }
        }
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
    fn atomic_replacement_creation_mode_and_bounds() {
        let (tmp, dir) = fixture();
        let name = OsStr::new("state.json");
        assert!(matches!(
            dir.replace(name, b"old", false),
            CommitOutcome::DurablyCommitted
        ));
        let outcome = dir.replace_with(
            name,
            b"complete new",
            false,
            OsStr::new(".test.tmp"),
            |point| {
                if point == Point::AfterCreate {
                    assert_eq!(
                        std::fs::metadata(tmp.path().join(".test.tmp"))
                            .unwrap()
                            .permissions()
                            .mode()
                            & 0o7777,
                        0o600
                    );
                    assert_eq!(dir.read(name, 64).unwrap().unwrap(), b"old");
                }
                Ok(())
            },
        );
        assert!(matches!(outcome, CommitOutcome::DurablyCommitted));
        assert_eq!(dir.read(name, 64).unwrap().unwrap(), b"complete new");
        assert!(dir.read(name, 2).is_err());
        assert!(matches!(
            dir.replace(name, b"overwrite", true),
            CommitOutcome::NotCommitted(_)
        ));
    }

    #[test]
    fn deterministic_commit_failure_matrix() {
        for fail in [
            Point::BeforeCreate,
            Point::AfterCreate,
            Point::MidWrite,
            Point::BeforeFileSync,
            Point::BeforeRename,
            Point::AfterRename,
            Point::AfterDirectorySync,
        ] {
            let (_tmp, dir) = fixture();
            let name = OsStr::new("state.json");
            assert!(matches!(
                dir.replace(name, b"old", false),
                CommitOutcome::DurablyCommitted
            ));
            let outcome = dir.replace_with(
                name,
                b"new complete",
                false,
                OsStr::new(".test.tmp"),
                |point| {
                    if point == fail {
                        Err(io::Error::other("injected persistence failure"))
                    } else {
                        Ok(())
                    }
                },
            );
            let after_rename = matches!(fail, Point::AfterRename | Point::AfterDirectorySync);
            assert_eq!(
                matches!(outcome, CommitOutcome::IndeterminateAfterRename(_)),
                after_rename,
                "{fail:?}"
            );
            assert_eq!(
                dir.read(name, 64).unwrap().unwrap(),
                if after_rename {
                    b"new complete".as_slice()
                } else {
                    b"old".as_slice()
                }
            );
        }
    }

    #[test]
    fn collisions_links_and_special_files_preserve_victim() {
        let (tmp, dir) = fixture();
        assert!(matches!(
            dir.replace(OsStr::new("victim"), b"untouched", false),
            CommitOutcome::DurablyCommitted
        ));
        symlink(tmp.path().join("victim"), tmp.path().join("symlink")).unwrap();
        std::fs::hard_link(tmp.path().join("victim"), tmp.path().join("hardlink")).unwrap();
        std::fs::create_dir(tmp.path().join("directory")).unwrap();
        fs::mknodat(
            &dir.fd,
            "fifo",
            FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(tmp.path().join("socket")).unwrap();
        for name in ["symlink", "hardlink", "directory", "fifo", "socket"] {
            assert!(dir.read(OsStr::new(name), 1024).is_err(), "{name}");
            assert!(
                matches!(
                    dir.replace(OsStr::new(name), b"bad", false),
                    CommitOutcome::NotCommitted(_)
                ),
                "{name}"
            );
        }
        let outcome = dir.replace_with(
            OsStr::new("new"),
            b"new",
            false,
            OsStr::new("victim"),
            |_| Ok(()),
        );
        assert!(matches!(outcome, CommitOutcome::NotCommitted(_)));
        assert_eq!(
            std::fs::read(tmp.path().join("victim")).unwrap(),
            b"untouched"
        );
    }

    #[test]
    fn path_substitution_before_rename_is_rejected() {
        let (tmp, dir) = fixture();
        let name = OsStr::new("state");
        assert!(matches!(
            dir.replace(name, b"old", false),
            CommitOutcome::DurablyCommitted
        ));
        assert!(matches!(
            dir.replace(OsStr::new("substitute"), b"other", false),
            CommitOutcome::DurablyCommitted
        ));
        let outcome = dir.replace_with(name, b"new", false, OsStr::new(".test.tmp"), |point| {
            if point == Point::BeforeRename {
                std::fs::rename(tmp.path().join("substitute"), tmp.path().join("state"))?;
            }
            Ok(())
        });
        assert!(matches!(outcome, CommitOutcome::NotCommitted(_)));
        assert_eq!(dir.read(name, 64).unwrap().unwrap(), b"other");
    }
}
