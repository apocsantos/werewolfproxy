use crate::kbucket::{InsertOutcome, KBucket};
use crate::nodeid::{bucket_index, NodeId};
use crate::peer::Peer;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
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

    pub fn add_peer(&mut self, peer: Peer) -> InsertOutcome {
        if peer.id == self.local_id {
            return InsertOutcome::LocalNode;
        }

        let idx = bucket_index(&self.local_id, &peer.id);
        self.buckets[idx].insert(peer)
    }

    pub fn remove_peer(&mut self, id: &NodeId) -> Option<Peer> {
        if id == &self.local_id {
            return None;
        }

        self.buckets[bucket_index(&self.local_id, id)].remove(id)
    }

    pub fn bucket(&self, index: usize) -> Option<&KBucket> {
        self.buckets.get(index)
    }

    /// Peers ordered by full 256-bit XOR distance to `target`, closest first.
    pub fn nearest(&self, target: &NodeId, limit: usize) -> Vec<Peer> {
        let mut peers: Vec<Peer> = self
            .buckets
            .iter()
            .flat_map(|bucket| bucket.peers().iter().cloned())
            .collect();

        // Big-endian byte arrays compare in numeric order, and distinct IDs
        // have distinct distances, so the order is total and deterministic.
        peers.sort_by_key(|peer| target.xor_distance(&peer.id));

        peers.truncate(limit);
        peers
    }

    pub fn peer_count(&self) -> usize {
        self.buckets.iter().map(KBucket::len).sum()
    }

    pub fn all_peers(&self) -> Vec<Peer> {
        self.buckets
            .iter()
            .flat_map(|bucket| bucket.peers().iter().cloned())
            .collect()
    }

    pub fn healthy_peers(&self) -> Vec<Peer> {
        self.all_peers()
            .into_iter()
            .filter(|peer| peer.healthy)
            .collect()
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

        let peer_a =
            Peer::new("wolf-a", "127.0.0.1:9560").with_health(true, 80.0, 100.0, Some(5.0));
        let peer_b =
            Peer::new("wolf-b", "127.0.0.1:9561").with_health(true, 90.0, 100.0, Some(3.0));

        let mut table = RoutingTable::new(local);
        table.add_peer(peer_a);
        table.add_peer(peer_b);

        let nearest = table.nearest(&target, 1);

        assert_eq!(nearest.len(), 1);
    }

    fn peer_with_id(id: [u8; 32]) -> Peer {
        let mut peer = Peer::new(format!("wolf-{}", id[31]), "127.0.0.1:9560");
        peer.id = NodeId(id);
        peer
    }

    #[test]
    fn nearest_orders_by_full_xor_distance() {
        let mut table = RoutingTable::new(NodeId([0xff; 32]));
        // All three share bucket_index 0 from the target; only the full
        // distance separates them. Reputation must not affect the order.
        let mut far = [0u8; 32];
        far[0] = 0x80;
        far[31] = 0x07;
        let mut mid = [0u8; 32];
        mid[0] = 0x80;
        mid[31] = 0x03;
        let mut near = [0u8; 32];
        near[0] = 0x80;
        near[31] = 0x01;
        table.add_peer(peer_with_id(far).with_health(true, 99.0, 100.0, None));
        table.add_peer(peer_with_id(mid));
        table.add_peer(peer_with_id(near));

        let target = NodeId([0u8; 32]);
        let order: Vec<_> = table
            .nearest(&target, 3)
            .into_iter()
            .map(|peer| peer.id.0[31])
            .collect();

        assert_eq!(order, vec![0x01, 0x03, 0x07]);
    }

    #[test]
    fn peer_differing_only_in_last_bit_is_stored() {
        let local = [0u8; 32];
        let mut neighbour = [0u8; 32];
        neighbour[31] = 0x01;

        let mut table = RoutingTable::new(NodeId(local));

        assert!(matches!(
            table.add_peer(peer_with_id(neighbour)),
            InsertOutcome::Inserted
        ));
        assert!(matches!(
            table.add_peer(peer_with_id(local)),
            InsertOutcome::LocalNode
        ));
        assert_eq!(table.peer_count(), 1);
        assert!(table.remove_peer(&NodeId(neighbour)).is_some());
    }
}
