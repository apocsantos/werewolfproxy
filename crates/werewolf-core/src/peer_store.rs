use crate::nodeid::NodeId;
use crate::peer::Peer;
use crate::routing::RoutingTable;

use std::fs;
use std::io;
use std::path::Path;

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
