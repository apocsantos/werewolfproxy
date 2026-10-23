use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceKind {
    Tcp,
    Http,
    Https,
    Ssh,
}

/// A service's providers, in order, including the first (`peer`, `target`).
/// Stage27 decision 4a.
pub const MAX_PROVIDERS: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceDefinition {
    pub name: String,
    pub peer: String,
    pub target: String,
    pub kind: ServiceKind,
    /// Stage27: further providers tried in order when the first fails
    /// before its session is established. Omitted when empty, so an entry
    /// without fallbacks serializes exactly as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<ServiceProvider>,
}

/// One more provider of a service: a Pack member and the target on it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceProvider {
    pub peer: String,
    pub target: String,
}

impl ServiceDefinition {
    /// Every provider in operator order: the first, then the fallbacks.
    pub fn providers(&self) -> impl Iterator<Item = (&str, &str)> {
        std::iter::once((self.peer.as_str(), self.target.as_str())).chain(
            self.fallbacks
                .iter()
                .map(|provider| (provider.peer.as_str(), provider.target.as_str())),
        )
    }
}

pub fn load_services(path: &Path) -> io::Result<Vec<ServiceDefinition>> {
    let services: Vec<ServiceDefinition> = crate::state_file::read(path)?.unwrap_or_default();
    validate_services(&services)?;
    Ok(services)
}

pub fn save_services(path: &Path, services: &[ServiceDefinition]) -> io::Result<()> {
    validate_services(services)?;
    crate::state_file::write(path, services, false)
}

pub fn validate_services(services: &[ServiceDefinition]) -> io::Result<()> {
    for service in services {
        validate_providers(service)?;
        if service.name.trim().is_empty()
            || service.peer.trim().is_empty()
            || service.target.trim().is_empty()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "service fields must not be empty",
            ));
        }

        if service.name != service.name.trim()
            || service.peer != service.peer.trim()
            || service.target != service.target.trim()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "service fields must be canonical",
            ));
        }

        if service.name.len() > 128 || service.peer.len() > 128 || service.target.len() > 512 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "service field exceeds limit",
            ));
        }
    }

    for (index, service) in services.iter().enumerate() {
        if services[..index]
            .iter()
            .any(|existing| existing.name == service.name)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "duplicate service name",
            ));
        }
    }

    Ok(())
}

/// The Stage23 field rules for every fallback, at most `MAX_PROVIDERS` in
/// all, and no peer listed twice: a second attempt at the same peer would
/// add nothing but another OPEN it can see.
fn validate_providers(service: &ServiceDefinition) -> io::Result<()> {
    let invalid = |message: &str| {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            message.to_owned(),
        ))
    };
    if service.fallbacks.len() + 1 > MAX_PROVIDERS {
        return invalid("too many service providers");
    }
    for provider in &service.fallbacks {
        if provider.peer.trim().is_empty()
            || provider.target.trim().is_empty()
            || provider.peer != provider.peer.trim()
            || provider.target != provider.target.trim()
            || provider.peer.len() > 128
            || provider.target.len() > 512
        {
            return invalid("service provider fields must be non-empty and canonical");
        }
    }
    let peers: Vec<&str> = service.providers().map(|(peer, _)| peer).collect();
    if peers
        .iter()
        .enumerate()
        .any(|(index, peer)| peers[..index].contains(peer))
    {
        return invalid("a service lists a provider twice");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str) -> ServiceDefinition {
        ServiceDefinition {
            name: name.into(),
            peer: "wolf-a".into(),
            target: "127.0.0.1:8080".into(),
            kind: ServiceKind::Http,
            fallbacks: Vec::new(),
        }
    }

    fn provider(peer: &str) -> ServiceProvider {
        ServiceProvider {
            peer: peer.into(),
            target: "127.0.0.1:8080".into(),
        }
    }

    #[test]
    fn an_entry_without_fallbacks_keeps_its_stage23_form() {
        let json = serde_json::to_string(&service("web")).unwrap();
        assert_eq!(
            json,
            r#"{"name":"web","peer":"wolf-a","target":"127.0.0.1:8080","kind":"http"}"#
        );
        assert_eq!(
            serde_json::from_str::<ServiceDefinition>(&json).unwrap(),
            service("web")
        );
    }

    #[test]
    fn fallbacks_round_trip_in_order() {
        let mut web = service("web");
        web.fallbacks = vec![provider("wolf-b"), provider("wolf-c")];
        assert!(validate_services(&[web.clone()]).is_ok());
        let json = serde_json::to_string(&web).unwrap();
        assert!(json.ends_with(
            r#""fallbacks":[{"peer":"wolf-b","target":"127.0.0.1:8080"},{"peer":"wolf-c","target":"127.0.0.1:8080"}]}"#
        ));
        assert_eq!(
            serde_json::from_str::<ServiceDefinition>(&json).unwrap(),
            web
        );
        let order: Vec<_> = web.providers().map(|(peer, _)| peer).collect();
        assert_eq!(order, ["wolf-a", "wolf-b", "wolf-c"]);
        // Unknown fields stay rejected, in an entry and in a provider.
        assert!(serde_json::from_str::<ServiceDefinition>(
            r#"{"name":"web","peer":"wolf-a","target":"127.0.0.1:8080","kind":"http","fallbacks":[{"peer":"wolf-b","target":"127.0.0.1:1","kind":"tcp"}]}"#
        )
        .is_err());
    }

    #[test]
    fn providers_are_bounded_unique_and_canonical() {
        let mut web = service("web");
        web.fallbacks = vec![provider("wolf-b"), provider("wolf-c"), provider("wolf-d")];
        assert!(validate_services(&[web.clone()]).is_ok());
        web.fallbacks.push(provider("wolf-e"));
        assert!(validate_services(&[web.clone()]).is_err(), "five providers");
        for fallbacks in [
            vec![provider("wolf-a")],
            vec![provider("wolf-b"), provider("wolf-b")],
            vec![provider("")],
            vec![provider(" wolf-b")],
            vec![ServiceProvider {
                peer: "wolf-b".into(),
                target: String::new(),
            }],
        ] {
            let mut candidate = service("web");
            candidate.fallbacks = fallbacks.clone();
            assert!(validate_services(&[candidate]).is_err(), "{fallbacks:?}");
        }
    }

    #[test]
    fn valid_service_document_is_accepted() {
        assert!(validate_services(&[service("web")]).is_ok());
    }

    #[test]
    fn duplicate_service_names_are_rejected() {
        assert!(validate_services(&[service("web"), service("web")]).is_err());
    }

    #[test]
    fn surrounding_whitespace_is_rejected() {
        let mut candidate = service("web");
        candidate.name = " web ".into();
        assert!(validate_services(&[candidate]).is_err());
    }

    #[test]
    fn empty_fields_are_rejected() {
        let mut candidate = service("web");
        candidate.peer.clear();
        assert!(validate_services(&[candidate]).is_err());
    }
}
