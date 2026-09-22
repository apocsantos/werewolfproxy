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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceDefinition {
    pub name: String,
    pub peer: String,
    pub target: String,
    pub kind: ServiceKind,
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
        if service.name.trim().is_empty()
            || service.peer.trim().is_empty()
            || service.target.trim().is_empty()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "service fields must not be empty",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str) -> ServiceDefinition {
        ServiceDefinition {
            name: name.into(),
            peer: "wolf-a".into(),
            target: "127.0.0.1:8080".into(),
            kind: ServiceKind::Http,
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
    fn empty_fields_are_rejected() {
        let mut candidate = service("web");
        candidate.peer.clear();
        assert!(validate_services(&[candidate]).is_err());
    }
}
