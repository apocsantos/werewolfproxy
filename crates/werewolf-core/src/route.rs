//! Stage26 routes (`docs/STAGE26_MULTIHOP_ROUTING_DESIGN.md`).
//!
//! A route says that Pack member `peer` is reached through Pack member
//! `via` (the relay). It changes only how bytes travel, never who `peer` is
//! or what it allows: the session to `peer` stays end to end, pinned to its
//! Pack key and authorized by its own Target Policy, and the relay forwards
//! only under its own Target Policy. Decisions 2a and 3a: routes live in
//! `routes.json`, outside the protected manifest, with exactly one relay.

use serde::{Deserialize, Serialize};

/// One route per destination; bounded like a Pack's worth of peers.
pub const MAX_ROUTES: usize = 256;
/// Same limit as a Pack member or service name.
pub const MAX_NAME_LEN: usize = 128;

/// Transport of the hop from this node to the relay (decision 4a). The
/// session inside it, to the destination, is always encrypted TCP.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RouteHop {
    Quic,
    Tcp,
}

impl RouteHop {
    pub fn as_str(self) -> &'static str {
        match self {
            RouteHop::Quic => "quic",
            RouteHop::Tcp => "tcp",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "quic" => Some(RouteHop::Quic),
            "tcp" => Some(RouteHop::Tcp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub peer: String,
    pub via: String,
    pub hop: RouteHop,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RouteError {
    #[error("route peer and relay names must be 1 to {MAX_NAME_LEN} bytes and canonical")]
    Name,
    #[error("a peer cannot be its own relay")]
    SelfRelay,
    #[error("a relay cannot itself be routed (exactly one relay)")]
    Chained,
    #[error("at most one route per peer")]
    Duplicate,
    #[error("at most {MAX_ROUTES} routes")]
    TooMany,
}

impl Route {
    pub fn check(&self) -> Result<(), RouteError> {
        check_name(&self.peer)?;
        check_name(&self.via)?;
        if self.peer == self.via {
            return Err(RouteError::SelfRelay);
        }
        Ok(())
    }
}

/// Validates a node's whole route list: each entry, one route per peer, and
/// no relay that is itself reached through a relay.
pub fn validate_routes(routes: &[Route]) -> Result<(), RouteError> {
    if routes.len() > MAX_ROUTES {
        return Err(RouteError::TooMany);
    }
    for (index, route) in routes.iter().enumerate() {
        route.check()?;
        if routes[..index].iter().any(|r| r.peer == route.peer) {
            return Err(RouteError::Duplicate);
        }
    }
    if routes
        .iter()
        .any(|route| routes.iter().any(|other| other.peer == route.via))
    {
        return Err(RouteError::Chained);
    }
    Ok(())
}

/// The Stage23 name rules, plus no control characters.
fn check_name(name: &str) -> Result<(), RouteError> {
    if name.is_empty()
        || name.len() > MAX_NAME_LEN
        || name != name.trim()
        || name.chars().any(char::is_control)
    {
        return Err(RouteError::Name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(peer: &str, via: &str) -> Route {
        Route {
            peer: peer.into(),
            via: via.into(),
            hop: RouteHop::Quic,
        }
    }

    #[test]
    fn names_must_be_canonical_bounded_and_distinct() {
        assert_eq!(route("d", "b").check(), Ok(()));
        for (peer, via) in [
            ("", "b"),
            ("d", ""),
            (" d", "b"),
            ("d", "b "),
            ("d\n", "b"),
            (&"n".repeat(MAX_NAME_LEN + 1), "b"),
        ] {
            assert_eq!(
                route(peer, via).check(),
                Err(RouteError::Name),
                "{peer:?} {via:?}"
            );
        }
        assert_eq!(route("d", "d").check(), Err(RouteError::SelfRelay));
    }

    #[test]
    fn one_route_per_peer_one_relay_and_bounded() {
        assert_eq!(validate_routes(&[]), Ok(()));
        assert_eq!(validate_routes(&[route("d", "b"), route("e", "b")]), Ok(()));
        assert_eq!(
            validate_routes(&[route("d", "b"), route("d", "c")]),
            Err(RouteError::Duplicate)
        );
        // b is reached through c, so b cannot relay for d (decision 3a).
        assert_eq!(
            validate_routes(&[route("d", "b"), route("b", "c")]),
            Err(RouteError::Chained)
        );
        assert_eq!(
            validate_routes(&[route("b", "c"), route("d", "b")]),
            Err(RouteError::Chained)
        );
        let many: Vec<_> = (0..=MAX_ROUTES)
            .map(|i| route(&format!("p{i}"), "relay"))
            .collect();
        assert_eq!(validate_routes(&many[..MAX_ROUTES]), Ok(()));
        assert_eq!(validate_routes(&many), Err(RouteError::TooMany));
    }

    #[test]
    fn json_form_is_strict() {
        let json = serde_json::to_string(&Route {
            peer: "d".into(),
            via: "b".into(),
            hop: RouteHop::Tcp,
        })
        .unwrap();
        assert_eq!(json, r#"{"peer":"d","via":"b","hop":"tcp"}"#);
        for bad in [
            r#"{"peer":"d","via":"b","hop":"tcp-plain"}"#,
            r#"{"peer":"d","via":"b"}"#,
            r#"{"peer":"d","via":"b","hop":"quic","target":"x"}"#,
        ] {
            assert!(serde_json::from_str::<Route>(bad).is_err(), "{bad}");
        }
        assert_eq!(RouteHop::parse("quic"), Some(RouteHop::Quic));
        assert_eq!(RouteHop::parse("tcp-plain"), None);
    }
}
