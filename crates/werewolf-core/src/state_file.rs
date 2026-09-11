//! Private adapter for legacy model APIs. Daemons use the retained-directory
//! commit-outcome API directly. These standalone writers also hold the Den lock.
use serde::{de::DeserializeOwned, Serialize};
use std::{io, path::Path};

#[cfg(target_os = "linux")]
fn directory(path: &Path) -> io::Result<(crate::local_fs::PrivateDirectory, std::ffi::OsString)> {
    let absolute = std::path::absolute(path)?;
    let name = absolute
        .file_name()
        .ok_or_else(|| io::Error::other("invalid state path"))?
        .to_owned();
    let parent = absolute
        .parent()
        .ok_or_else(|| io::Error::other("invalid state parent"))?;
    Ok((
        crate::local_fs::PrivateDirectory::open(parent, false)?,
        name,
    ))
}

pub(crate) fn read<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    #[cfg(target_os = "linux")]
    {
        let (directory, name) = directory(path)?;
        directory
            .read(&name, 1024 * 1024)?
            .map(|data| {
                serde_json::from_slice(&data).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid persistent state")
                })
            })
            .transpose()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "secure state requires Linux",
        ))
    }
}

pub(crate) fn write<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    create_only: bool,
) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use crate::local_fs::CommitOutcome;
        let (directory, name) = directory(path)?;
        let _writer = directory.lock(std::ffi::OsStr::new(".den.lock"))?;
        let data = serde_json::to_vec_pretty(value)
            .map_err(|_| io::Error::other("state serialization failed"))?;
        match directory.replace(&name, &data, create_only) {
            CommitOutcome::DurablyCommitted => Ok(()),
            CommitOutcome::NotCommitted(e) => Err(e),
            // This error must never be interpreted as a successful rollback.
            CommitOutcome::IndeterminateAfterRename(_) => Err(io::Error::other(
                "IndeterminateAfterRename: operator intervention required",
            )),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, value, create_only);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "secure state requires Linux",
        ))
    }
}
