use crate::kbucket::KBucket;
use crate::nodeid::{bucket_index, NodeId};
use crate::peer::Peer;

#[derive(Clone, Debug)]
pub struct RoutingTable {
    local_id: NodeId,
    buckets: Vec<KBucket>,
}

impl RoutingTable {
    pub fn new(local_id: NodeId) -> Self {
        Self {
            local_id,
            buckets: vec![KBucket::new(); 256],
        }
    }

    pub fn local_id(&self) -> &NodeId {
        &self.local_id
    }

    pub fn add_peer(&mut self, peer: Peer) {
        let idx = bucket_index(&self.local_id, &peer.id);

        if idx == 255 {
            return;
        }

        self.buckets[idx].insert(peer);
    }

    pub fn remove_peer(&mut self, id: &NodeId) -> Option<Peer> {
        let idx = bucket_index(&self.local_id, id);

        if idx == 255 {
            return None;
        }

        self.buckets[idx].remove(id)
    }

    pub fn bucket(&self, index: usize) -> Option<&KBucket> {
        self.buckets.get(index)
    }

    pub fn nearest(&self, target: &NodeId, limit: usize) -> Vec<Peer> {
        let mut peers: Vec<Peer> = self
            .buckets
            .iter()
            .flat_map(|bucket| bucket.peers().iter().cloned())
            .collect();

        peers.sort_by(|a, b| {
            let da = bucket_index(target, &a.id);
            let db = bucket_index(target, &b.id);

            da.cmp(&db)
                .then_with(|| b.reputation.total_cmp(&a.reputation))
        });

        peers.truncate(limit);
        peers
    }

    pub fn peer_count(&self) -> usize {
        self.buckets.iter().map(KBucket::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_peer_places_peer_in_table() {
        let local = NodeId::from_name_address("wolf-local", "127.0.0.1:9000");
        let peer = Peer::new("wolf-a", "127.0.0.1:9560");

        let mut table = RoutingTable::new(local);
        table.add_peer(peer);

        assert_eq!(table.peer_count(), 1);
    }

    #[test]
    fn nearest_returns_peers() {
        let local = NodeId::from_name_address("wolf-local", "127.0.0.1:9000");
        let target = NodeId::from_name_address("target", "key");

        let peer_a = Peer::new("wolf-a", "127.0.0.1:9560").with_health(true, 80.0, 100.0, Some(5.0));
        let peer_b = Peer::new("wolf-b", "127.0.0.1:9561").with_health(true, 90.0, 100.0, Some(3.0));

        let mut table = RoutingTable::new(local);
        table.add_peer(peer_a);
        table.add_peer(peer_b);

        let nearest = table.nearest(&target, 1);

        assert_eq!(nearest.len(), 1);
    }
}
