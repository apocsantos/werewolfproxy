use std::{
    collections::{BTreeSet, HashMap},
    net::{IpAddr, SocketAddr},
    path::Path,
};
use tokio::net::lookup_host;

#[derive(Clone, Debug, Default)]
pub(super) enum TargetPolicy {
    #[default]
    Deny,
    LegacyAllow,
    Grants(HashMap<String, BTreeSet<SocketAddr>>),
    Invalid,
}

pub(super) fn load(path: &Path) -> TargetPolicy {
    if !path.exists() {
        return TargetPolicy::Deny;
    }
    let data = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return TargetPolicy::Invalid,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return TargetPolicy::Invalid,
    };
    let root = match parsed.as_object() {
        Some(v) => v,
        None => return TargetPolicy::Invalid,
    };
    if root.keys().any(|k| k != "mode" && k != "peers") {
        return TargetPolicy::Invalid;
    }
    let mode = match parsed.get("mode").and_then(|v| v.as_str()) {
        Some(v) => v,
        None => return TargetPolicy::Invalid,
    };
    let peers_value = parsed.get("peers");
    match mode {
        "legacy-allow" if peers_value.is_none() => TargetPolicy::LegacyAllow,
        "deny-by-default" => {
            let mut grants = HashMap::new();
            let peers = match peers_value.and_then(|v| v.as_object()) {
                Some(v) => v,
                None => return TargetPolicy::Invalid,
            };
            for (fp, peer) in peers {
                if !valid_fingerprint(fp) {
                    return TargetPolicy::Invalid;
                }
                let peer_obj = match peer.as_object() {
                    Some(v) => v,
                    None => return TargetPolicy::Invalid,
                };
                if peer_obj.keys().any(|k| k != "targets") {
                    return TargetPolicy::Invalid;
                }
                let targets = match peer.get("targets").and_then(|v| v.as_array()) {
                    Some(v) => v,
                    None => return TargetPolicy::Invalid,
                };
                let mut set = BTreeSet::new();
                for target in targets {
                    let target_obj = match target.as_object() {
                        Some(v) => v,
                        None => return TargetPolicy::Invalid,
                    };
                    if target_obj.keys().any(|k| k != "address" && k != "port") {
                        return TargetPolicy::Invalid;
                    }
                    let address = match target.get("address").and_then(|v| v.as_str()) {
                        Some(v) => v,
                        None => return TargetPolicy::Invalid,
                    };
                    let port = match target
                        .get("port")
                        .and_then(|v| v.as_u64())
                        .and_then(|v| u16::try_from(v).ok())
                    {
                        Some(v) if v != 0 => v,
                        _ => return TargetPolicy::Invalid,
                    };
                    let ip: IpAddr = match address.parse() {
                        Ok(v) => v,
                        Err(_) => return TargetPolicy::Invalid,
                    };
                    let endpoint = SocketAddr::new(ip, port);
                    if !set.insert(endpoint) {
                        return TargetPolicy::Invalid;
                    }
                }
                grants.insert(fp.to_string(), set);
            }
            TargetPolicy::Grants(grants)
        }
        _ => TargetPolicy::Invalid,
    }
}

fn valid_fingerprint(fp: &str) -> bool {
    fp.starts_with("wwp1:") && fp.len() > 5
}

pub(super) async fn authorize(
    policy: &TargetPolicy,
    peer: &str,
    target: &str,
) -> Result<Vec<SocketAddr>, String> {
    let endpoints = resolve_target(target).await?;
    match policy {
        TargetPolicy::LegacyAllow => Ok(endpoints),
        TargetPolicy::Grants(grants)
            if grants
                .get(peer)
                .map(|allowed| endpoints.iter().all(|e| allowed.contains(e)))
                .unwrap_or(false) =>
        {
            Ok(endpoints)
        }
        _ => Err("target authorization denied".to_string()),
    }
}

async fn resolve_target(target: &str) -> Result<Vec<SocketAddr>, String> {
    let parsed = if target.parse::<SocketAddr>().is_ok() {
        vec![target
            .parse::<SocketAddr>()
            .map_err(|_| "invalid target".to_string())?]
    } else {
        let mut set = BTreeSet::new();
        for endpoint in lookup_host(target)
            .await
            .map_err(|_| "target resolution failed".to_string())?
        {
            set.insert(endpoint);
        }
        set.into_iter().collect()
    };
    if parsed.is_empty() {
        Err("target resolution failed".to_string())
    } else {
        Ok(parsed)
    }
}
