//! Linux local filesystem authority primitives. Same-UID attackers and root are
//! outside this boundary. Mode checks do not constitute a full POSIX ACL policy.
mod directory;
pub use directory::{effective_uid, PrivateDirectory};
