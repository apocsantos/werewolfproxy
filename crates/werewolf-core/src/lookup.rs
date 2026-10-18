use crate::nodeid::NodeId;
use crate::peer::Peer;
use crate::routing::RoutingTable;

/// Upper bound on peers returned for one lookup or FindNode request.
pub const MAX_K: usize = 20;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LookupResult {
    pub target: NodeId,
    pub requested: usize,
    pub returned: usize,
    pub peers: Vec<Peer>,
}

/// Returns at most `min(k, MAX_K)` peers closest to `target`; `requested`
/// records the caller's `k` unchanged.
pub fn lookup(table: &RoutingTable, target: &NodeId, k: usize) -> LookupResult {
    let peers = table.nearest(target, k.min(MAX_K));

    LookupResult {
        target: target.clone(),
        requested: k,
        returned: peers.len(),
        peers,
    }
}
