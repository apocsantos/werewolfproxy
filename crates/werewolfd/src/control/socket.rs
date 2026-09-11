use std::{ffi::OsString, fs::File, io, path::Path, time::Duration};
use tokio::net::{UnixListener, UnixStream};
use werewolf_core::local_fs::{effective_uid, PrivateDirectory};

/// Only constructed after socket finalization. Directory and instance lock live
/// at least as long as the listener. Dropping never unlinks an arbitrary path.
pub(crate) struct ControlListener {
    listener: UnixListener,
    _lock: File,
    _directory: PrivateDirectory,
}

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "control endpoint unavailable",
    )
}

pub(crate) async fn bind(socket: &str) -> io::Result<ControlListener> {
    let path = Path::new(socket);
    let parent = path.parent().ok_or_else(denied)?;
    let name = path.file_name().ok_or_else(denied)?;
    let directory = PrivateDirectory::open(parent, true)?;
    let mut lock_name = OsString::from(".");
    lock_name.push(name);
    lock_name.push(".lock");
    let lock = directory.lock(&lock_name)?;
    let pinned = directory.socket_path(name)?;
    if let Some(inode) = directory.socket_inode(name)? {
        match tokio::time::timeout(Duration::from_secs(1), UnixStream::connect(&pinned)).await {
            Ok(Err(e)) if e.kind() == io::ErrorKind::ConnectionRefused => {
                directory.unlink_stale_socket(name, inode)?;
            }
            // Success is active; timeout and every other error are ambiguous.
            _ => return Err(denied()),
        }
    }
    let listener = UnixListener::bind(&pinned)?;
    directory.restrict_socket(name)?;
    Ok(ControlListener {
        listener,
        _lock: lock,
        _directory: directory,
    })
}

impl ControlListener {
    pub(super) async fn accept(&self) -> io::Result<UnixStream> {
        let (stream, _) = self.listener.accept().await?;
        Ok(stream)
    }
}

/// All commands share ADMIN authority in 11B. Kernel credentials, not request
/// fields or socket ownership, authorize clients. Root has no separate bypass.
/// Same-UID applications cannot be isolated by this model.
pub(super) fn authorized(stream: &UnixStream) -> bool {
    authorize_uid(stream.peer_cred().map(|c| c.uid()), effective_uid())
}

fn authorize_uid(peer: io::Result<u32>, daemon: u32) -> bool {
    matches!(peer, Ok(uid) if uid == daemon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_policy_fails_closed_without_root_bypass() {
        assert!(authorize_uid(Ok(1000), 1000));
        assert!(!authorize_uid(Ok(1001), 1000));
        assert!(!authorize_uid(Ok(0), 1000));
        assert!(!authorize_uid(Err(denied()), 1000));
        assert!(authorize_uid(Ok(0), 0));
    }
}
