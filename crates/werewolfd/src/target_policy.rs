use serde::{
    de::{self, MapAccess, Visitor},
    Deserialize, Deserializer,
};
use std::{
    collections::{BTreeSet, HashMap},
    future::Future,
    net::{IpAddr, SocketAddr},
    path::Path,
};

#[derive(Clone, Debug, Default)]
pub(super) enum TargetPolicy {
    #[default]
    Deny,
    LegacyAllow,
    Grants(HashMap<String, BTreeSet<SocketAddr>>),
    Invalid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    mode: String,
    #[serde(default)]
    peers: Peers,
}

#[derive(Default)]
enum Peers {
    #[default]
    Missing,
    Present(HashMap<String, PeerGrant>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerGrant {
    targets: Vec<TargetGrant>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetGrant {
    address: IpAddr,
    port: u16,
}

// Unlike a plain HashMap/Value deserializer, reject duplicate peer keys.
// Derived struct deserializers also reject duplicate fields at every other level.
impl<'de> Deserialize<'de> for Peers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PeersVisitor;
        impl<'de> Visitor<'de> for PeersVisitor {
            type Value = Peers;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an object of unique canonical fingerprints")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Peers, M::Error> {
                let mut peers = HashMap::new();
                while let Some((fingerprint, grant)) = map.next_entry::<String, PeerGrant>()? {
                    if !valid_fingerprint(&fingerprint)
                        || peers.insert(fingerprint, grant).is_some()
                    {
                        return Err(de::Error::custom("invalid or duplicate peer"));
                    }
                }
                Ok(Peers::Present(peers))
            }
        }
        deserializer.deserialize_map(PeersVisitor)
    }
}

pub(super) fn load(path: &Path) -> TargetPolicy {
    let data = match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return TargetPolicy::Deny,
        Err(_) => return TargetPolicy::Invalid,
    };
    parse(&data)
}

pub(super) fn parse(data: &str) -> TargetPolicy {
    let parsed: PolicyFile = match serde_json::from_str(data) {
        Ok(value) => value,
        Err(_) => return TargetPolicy::Invalid,
    };
    match (parsed.mode.as_str(), parsed.peers) {
        ("legacy-allow", Peers::Missing) => TargetPolicy::LegacyAllow,
        ("deny-by-default", Peers::Present(peers)) => {
            let mut grants = HashMap::new();
            for (fingerprint, peer) in peers {
                let mut endpoints = BTreeSet::new();
                for target in peer.targets {
                    let endpoint = SocketAddr::new(target.address, target.port);
                    if !supported_endpoint(&endpoint) || !endpoints.insert(endpoint) {
                        return TargetPolicy::Invalid;
                    }
                }
                grants.insert(fingerprint, endpoints);
            }
            TargetPolicy::Grants(grants)
        }
        _ => TargetPolicy::Invalid,
    }
}

fn valid_fingerprint(fp: &str) -> bool {
    fp.strip_prefix("wwp1:").is_some_and(|suffix| {
        suffix.len() == 23
            && suffix.bytes().enumerate().all(|(i, b)| {
                if i % 3 == 2 {
                    b == b'-'
                } else {
                    b.is_ascii_digit() || (b'A'..=b'F').contains(&b)
                }
            })
    })
}

fn supported_endpoint(endpoint: &SocketAddr) -> bool {
    endpoint.port() != 0
        && match endpoint {
            SocketAddr::V4(_) => true,
            // Grants cannot represent a scope. Reject link-local even with no scope
            // instead of relying on OS interface selection. Never strip a scope ID.
            SocketAddr::V6(v6) => v6.scope_id() == 0 && !v6.ip().is_unicast_link_local(),
        }
}

pub(super) async fn authorize(
    policy: &TargetPolicy,
    peer: &str,
    target: &str,
) -> Result<Vec<SocketAddr>, String> {
    authorize_with_resolver(policy, peer, target, |host, port| async move {
        tokio::net::lookup_host((host.as_str(), port))
            .await
            .map(|addresses| addresses.collect())
            .map_err(|_| ())
    })
    .await
}

async fn authorize_with_resolver<F, Fut>(
    policy: &TargetPolicy,
    peer: &str,
    target: &str,
    resolver: F,
) -> Result<Vec<SocketAddr>, String>
where
    F: FnOnce(String, u16) -> Fut,
    Fut: Future<Output = Result<Vec<SocketAddr>, ()>>,
{
    const DENIED: &str = "target authorization denied";
    if matches!(policy, TargetPolicy::Deny | TargetPolicy::Invalid) || target.contains('%') {
        return Err(DENIED.into());
    }
    let endpoints = if let Ok(endpoint) = target.parse::<SocketAddr>() {
        vec![endpoint]
    } else {
        let (host, port) = target.rsplit_once(':').ok_or(DENIED)?;
        let port = port.parse::<u16>().map_err(|_| DENIED)?;
        if host.is_empty()
            || host.contains([':', '[', ']'])
            || host.chars().any(char::is_whitespace)
            || port == 0
        {
            return Err(DENIED.into());
        }
        // This is the sole DNS call for an authorization attempt.
        resolver(host.to_owned(), port).await.map_err(|_| DENIED)?
    };
    let endpoints: BTreeSet<_> = endpoints.into_iter().collect();
    if endpoints.is_empty() || endpoints.iter().any(|e| !supported_endpoint(e)) {
        return Err(DENIED.into());
    }
    match policy {
        TargetPolicy::LegacyAllow => Ok(endpoints.into_iter().collect()),
        TargetPolicy::Grants(grants)
            if grants
                .get(peer)
                .is_some_and(|allowed| endpoints.is_subset(allowed)) =>
        {
            Ok(endpoints.into_iter().collect())
        }
        _ => Err(DENIED.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_policy(name: &str, body: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("werewolf-policy-{}-{}", name, std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn missing_policy_denies() {
        let path =
            std::env::temp_dir().join(format!("werewolf-policy-missing-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(matches!(load(&path), TargetPolicy::Deny));
    }

    #[test]
    fn malformed_policy_is_invalid() {
        let path = temp_policy("malformed", "{");
        assert!(matches!(load(&path), TargetPolicy::Invalid));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn exact_ip_and_port_grant_authorizes_without_connecting() {
        let path = temp_policy(
            "grant",
            r#"{"mode":"deny-by-default","peers":{"wwp1:01-23-45-67-89-AB-CD-EF":{"targets":[{"address":"127.0.0.1","port":1}]}}}"#,
        );
        let policy = load(&path);
        let endpoints = authorize(&policy, "wwp1:01-23-45-67-89-AB-CD-EF", "127.0.0.1:1")
            .await
            .unwrap();
        assert_eq!(endpoints, vec!["127.0.0.1:1".parse().unwrap()]);
        assert!(
            authorize(&policy, "wwp1:FE-DC-BA-98-76-54-32-10", "127.0.0.1:1")
                .await
                .is_err()
        );
        let _ = std::fs::remove_file(path);
    }

    const PEER: &str = "wwp1:01-23-45-67-89-AB-CD-EF";

    fn grants(addresses: &[&str]) -> TargetPolicy {
        TargetPolicy::Grants(HashMap::from([(
            PEER.into(),
            addresses.iter().map(|a| a.parse().unwrap()).collect(),
        )]))
    }

    #[tokio::test]
    async fn literal_and_per_peer_matrix() {
        let policy = grants(&["127.0.0.1:8080", "[::1]:8080"]);
        for target in ["127.0.0.1:8080", "[0:0:0:0:0:0:0:1]:8080"] {
            assert_eq!(
                authorize(&policy, PEER, target).await.unwrap(),
                vec![target.parse::<SocketAddr>().unwrap()]
            );
        }
        for target in [
            "127.0.0.1:8081",
            "127.0.0.2:8080",
            "[::1]:8081",
            "10.1.2.3:8080",
            "172.16.0.1:8080",
            "192.168.1.1:8080",
            "169.254.1.1:8080",
            "1.1.1.1:8080",
            "[fd00::1]:8080",
            "[2001:db8::1]:8080",
        ] {
            assert!(authorize(&policy, PEER, target).await.is_err(), "{target}");
        }
        assert!(
            authorize(&policy, "wwp1:FE-DC-BA-98-76-54-32-10", "127.0.0.1:8080")
                .await
                .is_err()
        );
        assert!(authorize(&grants(&[]), PEER, "127.0.0.1:8080")
            .await
            .is_err());
    }

    #[test]
    fn malformed_configuration_matrix() {
        let valid = serde_json::json!({"mode":"deny-by-default", "peers":{PEER:{"targets":[{"address":"127.0.0.1", "port":8080}]}}});
        assert!(matches!(parse(&valid.to_string()), TargetPolicy::Grants(_)));
        for mode in ["unknown", "", "legacy-allow"] {
            let mut value = valid.clone();
            value["mode"] = mode.into();
            assert!(matches!(parse(&value.to_string()), TargetPolicy::Invalid));
        }
        for fingerprint in [
            "wwp1:test",
            "wwp1:",
            "wwp1:0123456789abcdeg",
            "wwp1:01-23-45-67-89-ab-cd-ef",
            "0123456789abcdef",
            "wwp1:01-23-45-67-89-AB-CD-EF0",
        ] {
            let value =
                serde_json::json!({"mode":"deny-by-default", "peers":{fingerprint:{"targets":[]}}});
            assert!(
                matches!(parse(&value.to_string()), TargetPolicy::Invalid),
                "{fingerprint}"
            );
        }
        for address in [
            "localhost",
            "127.0.0.1/32",
            "*",
            "not-an-ip",
            "fe80::1",
            "fe80::1%2",
        ] {
            let mut value = valid.clone();
            value["peers"][PEER]["targets"][0]["address"] = address.into();
            assert!(
                matches!(parse(&value.to_string()), TargetPolicy::Invalid),
                "{address}"
            );
        }
        for port in [
            serde_json::json!(0),
            serde_json::json!(-1),
            serde_json::json!(65536),
            serde_json::json!(1.5),
            serde_json::json!("8080"),
            serde_json::Value::Null,
        ] {
            let mut value = valid.clone();
            value["peers"][PEER]["targets"][0]["port"] = port;
            assert!(matches!(parse(&value.to_string()), TargetPolicy::Invalid));
        }
        for value in [
            serde_json::json!({"mode":"deny-by-default"}),
            serde_json::json!({"mode":"legacy-allow","peers":null}),
            serde_json::json!({"mode":"deny-by-default","peers":{},"rules":[]}),
            serde_json::json!({"mode":"deny-by-default","peers":{PEER:{"targets":[],"alias":"a"}}}),
            serde_json::json!({"mode":"deny-by-default","peers":{PEER:{"targets":[{"address":"127.0.0.1","port":8080,"cidr":32}]}}}),
        ] {
            assert!(matches!(parse(&value.to_string()), TargetPolicy::Invalid));
        }
    }

    #[test]
    fn duplicate_and_conflicting_entries_fail_closed() {
        for data in [
            r#"{"mode":"deny-by-default","mode":"legacy-allow"}"#,
            r#"{"mode":"deny-by-default","peers":{},"peers":{}}"#,
            r#"{"mode":"deny-by-default","peers":{"wwp1:01-23-45-67-89-AB-CD-EF":{"targets":[]},"wwp1:01-23-45-67-89-AB-CD-EF":{"targets":[]}}}"#,
            r#"{"mode":"deny-by-default","peers":{"wwp1:01-23-45-67-89-AB-CD-EF":{"targets":[],"targets":[]}}}"#,
            r#"{"mode":"deny-by-default","peers":{"wwp1:01-23-45-67-89-AB-CD-EF":{"targets":[{"address":"::1","address":"127.0.0.1","port":8080}]}}}"#,
            r#"{"mode":"deny-by-default","peers":{"wwp1:01-23-45-67-89-AB-CD-EF":{"targets":[{"address":"::1","port":8080,"port":8081}]}}}"#,
            r#"{"mode":"deny-by-default","peers":{"wwp1:01-23-45-67-89-AB-CD-EF":{"targets":[{"address":"::1","port":8080},{"address":"0:0:0:0:0:0:0:1","port":8080}]}}}"#,
        ] {
            assert!(matches!(parse(data), TargetPolicy::Invalid), "{data}");
        }
    }

    #[tokio::test]
    async fn missing_invalid_and_unknown_policy_deny_at_runtime() {
        for policy in [
            TargetPolicy::Deny,
            parse("{"),
            parse(r#"{"mode":"unknown"}"#),
        ] {
            assert_eq!(
                authorize(&policy, PEER, "127.0.0.1:8080")
                    .await
                    .unwrap_err(),
                "target authorization denied"
            );
        }
    }

    #[tokio::test]
    async fn dns_complete_mixed_empty_and_unauthorized_sets() {
        use std::cell::Cell;
        let policy = grants(&["127.0.0.1:8080", "[::1]:8080"]);
        for (addresses, allowed) in [
            (vec!["127.0.0.1:8080", "[::1]:8080", "127.0.0.1:8080"], true),
            (vec!["127.0.0.1:8080", "127.0.0.2:8080"], false),
            (vec!["127.0.0.2:8080"], false),
            (vec![], false),
        ] {
            let calls = Cell::new(0);
            let result =
                authorize_with_resolver(&policy, PEER, "fixture.invalid:8080", |host, port| {
                    calls.set(calls.get() + 1);
                    assert_eq!((host.as_str(), port), ("fixture.invalid", 8080));
                    async move { Ok(addresses.iter().map(|a| a.parse().unwrap()).collect()) }
                })
                .await;
            assert_eq!(calls.get(), 1);
            assert_eq!(result.is_ok(), allowed);
            if allowed {
                assert_eq!(result.unwrap().len(), 2);
            }
        }
    }

    #[tokio::test]
    async fn literal_bypasses_dns_and_resolver_failure_is_generic() {
        let policy = grants(&["127.0.0.1:8080"]);
        assert!(
            authorize_with_resolver(&policy, PEER, "127.0.0.1:8080", |_, _| async {
                panic!("literal must not resolve")
            })
            .await
            .is_ok()
        );
        assert_eq!(
            authorize_with_resolver(&policy, PEER, "fixture.invalid:8080", |_, _| async {
                Err(())
            })
            .await
            .unwrap_err(),
            "target authorization denied"
        );
    }

    #[tokio::test]
    async fn scoped_link_local_and_invalid_syntax_fail_closed_even_in_legacy() {
        for target in [
            "[fe80::1%2]:8080",
            "[fe80::1%lo]:8080",
            "[fe80::1]:8080",
            "[::1%0]:8080",
            "localhost:0",
            "localhost:65536",
            "localhost",
            ":8080",
            "::1:8080",
        ] {
            assert!(
                authorize(&TargetPolicy::LegacyAllow, PEER, target)
                    .await
                    .is_err(),
                "{target}"
            );
        }
        for endpoint in ["[fe80::1]:8080", "[::1%2]:8080"] {
            assert!(authorize_with_resolver(
                &TargetPolicy::LegacyAllow,
                PEER,
                "fixture.invalid:8080",
                |_, _| async move { Ok(vec![endpoint.parse().unwrap()]) }
            )
            .await
            .is_err());
        }
    }

    #[tokio::test]
    async fn explicit_legacy_allows_ungranted_endpoints() {
        let policy = parse(r#"{"mode":"legacy-allow"}"#);
        assert!(matches!(policy, TargetPolicy::LegacyAllow));
        for target in [
            "127.0.0.1:8080",
            "[::1]:8080",
            "10.0.0.1:8080",
            "1.1.1.1:8080",
        ] {
            assert!(authorize(&policy, PEER, target).await.is_ok());
        }
    }
}
