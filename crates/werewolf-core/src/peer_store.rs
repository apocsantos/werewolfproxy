use crate::nodeid::NodeId;
use crate::peer::Peer;
use crate::routing::RoutingTable;

use std::fs;
use std::io;
use std::path::Path;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PeerStoreSummary {
    pub peer_count: usize,
    pub healthy_count: usize,
    pub best_peer: Option<String>,
    pub best_reputation: Option<f64>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PeerStore {
    pub local_id: NodeId,
    pub peers: Vec<Peer>,
}

impl PeerStore {
    pub fn new(local_id: NodeId) -> Self {
        Self {
            local_id,
            peers: Vec::new(),
        }
    }

    pub fn from_routing_table(table: &RoutingTable) -> Self {
        Self {
            local_id: table.local_id().clone(),
            peers: table.all_peers(),
        }
    }

    pub fn into_routing_table(self) -> RoutingTable {
        let mut table = RoutingTable::new(self.local_id);

        for peer in self.peers {
            table.add_peer(peer);
        }

        table
    }

    pub fn summary(&self) -> PeerStoreSummary {
        let peer_count = self.peers.len();
        let healthy_count = self.peers.iter().filter(|p| p.healthy).count();

        let best = self
            .peers
            .iter()
            .max_by(|a, b| a.reputation.total_cmp(&b.reputation));

        PeerStoreSummary {
            peer_count,
            healthy_count,
            best_peer: best.map(|p| p.name.clone()),
            best_reputation: best.map(|p| p.reputation),
        }
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let data = fs::read_to_string(path)?;
        let store = serde_json::from_str(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        Ok(store)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        fs::write(path, data)
    }
}
