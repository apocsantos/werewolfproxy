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

fn parse(data: &str) -> TargetPolicy {
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
