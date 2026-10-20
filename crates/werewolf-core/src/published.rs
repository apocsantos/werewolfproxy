//! Services a node publishes for Stage25 service discovery
//! (`docs/STAGE25_SERVICE_DISCOVERY_DESIGN.md`).
//!
//! A published service says that this node offers `name` at `target`. It
//! grants nothing: a requester still needs the provider's Target Policy to
//! allow that target, and the provider shows an entry only to peers it has
//! already granted it. Entries travel inside a signed RPC result, so they
//! carry no signature of their own.

use crate::service::ServiceKind;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// At most this many services per node, so one answer stays bounded.
pub const MAX_PUBLISHED: usize = 32;
/// Same limit as a Stage23 service name.
pub const MAX_NAME_LEN: usize = 128;
/// Longest canonical `IP:port`: `[` + 39-character IPv6 + `]:` + 5 digits.
pub const MAX_TARGET_LEN: usize = 47;
/// Upper bound of one encoded entry, checked before any parsing.
pub const MAX_ENCODED_LEN: usize = 1 + 2 + MAX_NAME_LEN + 2 + MAX_TARGET_LEN;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublishedService {
    pub name: String,
    pub kind: ServiceKind,
    pub target: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PublishedError {
    #[error("published service name must be 1 to {MAX_NAME_LEN} bytes, canonical, without control characters")]
    Name,
    #[error("published service target must be a canonical literal IP:port")]
    Target,
    #[error("published service names must be unique")]
    Duplicate,
    #[error("at most {MAX_PUBLISHED} services may be published")]
    TooMany,
    #[error("published service encoding is malformed")]
    Malformed,
}

impl PublishedService {
    pub fn check(&self) -> Result<(), PublishedError> {
        check_name(&self.name)?;
        check_target(&self.target)
    }

    /// Canonical encoding: kind byte, then length-prefixed (u16, big-endian)
    /// name and target. Call only on a checked entry.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAX_ENCODED_LEN);
        out.push(kind_code(&self.kind));
        for field in [&self.name, &self.target] {
            out.extend_from_slice(&(field.len() as u16).to_be_bytes());
            out.extend_from_slice(field.as_bytes());
        }
        out
    }

    /// Strict inverse of `encode`: bounded, exact length, no trailing bytes,
    /// and the decoded entry must pass `check`.
    pub fn decode(bytes: &[u8]) -> Result<Self, PublishedError> {
        if bytes.len() > MAX_ENCODED_LEN {
            return Err(PublishedError::Malformed);
        }
        let (&code, mut rest) = bytes.split_first().ok_or(PublishedError::Malformed)?;
        let kind = kind_from_code(code).ok_or(PublishedError::Malformed)?;
        let mut field = |limit: usize| -> Result<String, PublishedError> {
            if rest.len() < 2 {
                return Err(PublishedError::Malformed);
            }
            let len = u16::from_be_bytes([rest[0], rest[1]]) as usize;
            if len > limit || rest.len() < 2 + len {
                return Err(PublishedError::Malformed);
            }
            let text = std::str::from_utf8(&rest[2..2 + len])
                .map_err(|_| PublishedError::Malformed)?
                .to_string();
            rest = &rest[2 + len..];
            Ok(text)
        };
        let name = field(MAX_NAME_LEN)?;
        let target = field(MAX_TARGET_LEN)?;
        if !rest.is_empty() {
            return Err(PublishedError::Malformed);
        }
        let entry = Self { name, kind, target };
        entry.check()?;
        Ok(entry)
    }
}

/// Validates a node's whole published list.
pub fn validate_published(services: &[PublishedService]) -> Result<(), PublishedError> {
    if services.len() > MAX_PUBLISHED {
        return Err(PublishedError::TooMany);
    }
    for (index, service) in services.iter().enumerate() {
        service.check()?;
        if services[..index].iter().any(|s| s.name == service.name) {
            return Err(PublishedError::Duplicate);
        }
    }
    Ok(())
}

/// The Stage23 rules (non-empty, no surrounding whitespace, bounded) plus no
/// control characters: a requester prints names that came from another node.
fn check_name(name: &str) -> Result<(), PublishedError> {
    if name.is_empty()
        || name.len() > MAX_NAME_LEN
        || name != name.trim()
        || name.chars().any(char::is_control)
    {
        return Err(PublishedError::Name);
    }
    Ok(())
}

/// A literal, specified IP with a non-zero port, written exactly as Rust
/// formats it, so each target has one spelling. Hostnames are rejected.
fn check_target(target: &str) -> Result<(), PublishedError> {
    let socket = target
        .parse::<SocketAddr>()
        .map_err(|_| PublishedError::Target)?;
    if socket.port() == 0 || socket.ip().is_unspecified() || socket.to_string() != target {
        return Err(PublishedError::Target);
    }
    Ok(())
}

fn kind_code(kind: &ServiceKind) -> u8 {
    match kind {
        ServiceKind::Tcp => 0,
        ServiceKind::Http => 1,
        ServiceKind::Https => 2,
        ServiceKind::Ssh => 3,
    }
}

fn kind_from_code(code: u8) -> Option<ServiceKind> {
    Some(match code {
        0 => ServiceKind::Tcp,
        1 => ServiceKind::Http,
        2 => ServiceKind::Https,
        3 => ServiceKind::Ssh,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: ServiceKind, target: &str) -> PublishedService {
        PublishedService {
            name: name.into(),
            kind,
            target: target.into(),
        }
    }

    fn web() -> PublishedService {
        entry("web", ServiceKind::Http, "127.0.0.1:8080")
    }

    #[test]
    fn encoding_is_canonical_and_round_trips() {
        let web = web();
        assert_eq!(
            web.encode(),
            [&[1u8, 0, 3][..], b"web", &[0, 14], b"127.0.0.1:8080"].concat()
        );
        assert_eq!(PublishedService::decode(&web.encode()), Ok(web));
        for kind in [ServiceKind::Tcp, ServiceKind::Https, ServiceKind::Ssh] {
            let other = entry("ssh-admin", kind, "[2001:db8::1]:22");
            assert_eq!(PublishedService::decode(&other.encode()), Ok(other));
        }
    }

    #[test]
    fn longest_entry_fits_the_encoded_bound() {
        let longest = entry(
            &"n".repeat(MAX_NAME_LEN),
            ServiceKind::Tcp,
            "[ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffe]:65535",
        );
        assert_eq!(longest.target.len(), MAX_TARGET_LEN);
        assert_eq!(longest.encode().len(), MAX_ENCODED_LEN);
        assert_eq!(PublishedService::decode(&longest.encode()), Ok(longest));
    }

    #[test]
    fn names_must_be_canonical_bounded_and_printable() {
        for name in [
            "",
            " web",
            "web ",
            "we\nb",
            "web\u{1b}[2J",
            "\u{7f}",
            &"n".repeat(MAX_NAME_LEN + 1),
        ] {
            let candidate = entry(name, ServiceKind::Tcp, "127.0.0.1:8080");
            assert_eq!(candidate.check(), Err(PublishedError::Name), "{name:?}");
        }
        assert!(entry("café 2", ServiceKind::Tcp, "127.0.0.1:8080")
            .check()
            .is_ok());
    }

    #[test]
    fn targets_must_be_canonical_literal_socket_addresses() {
        for target in [
            "localhost:8080",
            "127.0.0.1",
            "127.0.0.1:0",
            "0.0.0.0:8080",
            "[::]:22",
            "[2001:db8:0:0::1]:22",
            "127.0.0.1:08080",
            " 127.0.0.1:8080",
            "tcp://127.0.0.1:8080",
        ] {
            let candidate = entry("web", ServiceKind::Tcp, target);
            assert_eq!(candidate.check(), Err(PublishedError::Target), "{target}");
        }
    }

    #[test]
    fn decode_rejects_malformed_input() {
        let good = web().encode();
        // Unknown kind byte.
        let mut unknown = good.clone();
        unknown[0] = 4;
        assert_eq!(
            PublishedService::decode(&unknown),
            Err(PublishedError::Malformed)
        );
        // Truncated anywhere, and a trailing byte.
        for len in 0..good.len() {
            assert!(PublishedService::decode(&good[..len]).is_err(), "{len}");
        }
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(
            PublishedService::decode(&trailing),
            Err(PublishedError::Malformed)
        );
        // A length prefix past its field limit.
        let mut long_name = vec![0u8];
        long_name.extend_from_slice(&((MAX_NAME_LEN + 1) as u16).to_be_bytes());
        long_name.extend(std::iter::repeat_n(b'n', MAX_NAME_LEN + 1));
        assert_eq!(
            PublishedService::decode(&long_name),
            Err(PublishedError::Malformed)
        );
        // Invalid UTF-8.
        let bad_utf8 = [&[0u8, 0, 1][..], &[0xff], &[0, 14], b"127.0.0.1:8080"].concat();
        assert_eq!(
            PublishedService::decode(&bad_utf8),
            Err(PublishedError::Malformed)
        );
        // Oversized input is rejected before parsing.
        assert_eq!(
            PublishedService::decode(&[0u8; MAX_ENCODED_LEN + 1]),
            Err(PublishedError::Malformed)
        );
    }

    #[test]
    fn decode_applies_the_field_rules() {
        let control = entry("we\tb", ServiceKind::Tcp, "127.0.0.1:8080");
        assert_eq!(
            PublishedService::decode(&control.encode()),
            Err(PublishedError::Name)
        );
        let hostname = entry("web", ServiceKind::Tcp, "localhost:80");
        assert_eq!(
            PublishedService::decode(&hostname.encode()),
            Err(PublishedError::Target)
        );
    }

    #[test]
    fn list_rules_cap_count_and_require_unique_names() {
        assert_eq!(validate_published(&[]), Ok(()));
        assert_eq!(
            validate_published(&[web(), web()]),
            Err(PublishedError::Duplicate)
        );
        let many: Vec<_> = (0..=MAX_PUBLISHED)
            .map(|i| entry(&format!("s{i}"), ServiceKind::Tcp, "127.0.0.1:8080"))
            .collect();
        assert_eq!(validate_published(&many[..MAX_PUBLISHED]), Ok(()));
        assert_eq!(validate_published(&many), Err(PublishedError::TooMany));
        let bad = entry("bad", ServiceKind::Tcp, "127.0.0.1:0");
        assert_eq!(
            validate_published(&[web(), bad]),
            Err(PublishedError::Target)
        );
    }

    #[test]
    fn json_form_matches_the_stage23_kind_names() {
        let json = serde_json::to_string(&web()).unwrap();
        assert_eq!(
            json,
            r#"{"name":"web","kind":"http","target":"127.0.0.1:8080"}"#
        );
        assert!(serde_json::from_str::<PublishedService>(
            r#"{"name":"web","kind":"http","target":"127.0.0.1:8080","peer":"x"}"#
        )
        .is_err());
    }
}
