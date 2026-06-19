use crate::nodeid::NodeId;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Peer {
    pub id: NodeId,
    pub name: String,
    pub address: String,
    pub healthy: bool,
    pub reputation: f64,
    pub availability_pct: f64,
    pub latency_ms: Option<f64>,
}

impl Peer {
    pub fn new(name: impl Into<String>, address: impl Into<String>) -> Self {
        let name = name.into();
        let address = address.into();

        Self {
            id: NodeId::from_name_address(&name, &address),
            name,
            address,
            healthy: false,
            reputation: 0.0,
            availability_pct: 0.0,
            latency_ms: None,
        }
    }

    pub fn with_health(
        mut self,
        healthy: bool,
        reputation: f64,
        availability_pct: f64,
        latency_ms: Option<f64>,
    ) -> Self {
        self.healthy = healthy;
        self.reputation = reputation;
        self.availability_pct = availability_pct;
        self.latency_ms = latency_ms;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_gets_deterministic_id() {
        let a = Peer::new("wolf-a", "127.0.0.1:9560");
        let b = Peer::new("wolf-a", "127.0.0.1:9560");

        assert_eq!(a.id, b.id);
    }
}
