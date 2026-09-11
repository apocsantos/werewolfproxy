//! Linux local filesystem authority primitives. Same-UID attackers and root are
//! outside this boundary. Mode checks do not constitute a full POSIX ACL policy.
mod directory;
pub use directory::{effective_uid, PrivateDirectory};

/// Default Linux control endpoint. No shared-/tmp fallback is permitted.
/// An explicit --socket may instead name a socket in another private directory.
pub fn default_control_socket() -> std::io::Result<std::path::PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "set XDG_RUNTIME_DIR or explicitly configure a private control socket",
        )
    })?;
    let runtime = std::path::PathBuf::from(runtime);
    let _validated = PrivateDirectory::open(&runtime, false)?;
    Ok(runtime.join("werewolf").join("control.sock"))
}

mod atomic;
pub use atomic::CommitOutcome;
