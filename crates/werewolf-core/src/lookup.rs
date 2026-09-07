use crate::nodeid::NodeId;
use crate::peer::Peer;
use crate::routing::RoutingTable;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LookupResult {
    pub target: NodeId,
    pub requested: usize,
    pub returned: usize,
    pub peers: Vec<Peer>,
}

pub fn lookup(table: &RoutingTable, target: &NodeId, k: usize) -> LookupResult {
    let peers = table.nearest(target, k);

    LookupResult {
        target: target.clone(),
        requested: k,
        returned: peers.len(),
        peers,
    }
}
