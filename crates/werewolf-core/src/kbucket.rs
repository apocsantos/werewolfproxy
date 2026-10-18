use crate::nodeid::NodeId;
use crate::peer::Peer;

/// Kademlia bucket capacity.
pub const K: usize = 20;

#[derive(Clone, Debug)]
pub enum InsertOutcome {
    Inserted,
    /// Already present; refreshed and moved to the most-recently-seen end.
    Refreshed,
    /// The bucket is full. The caller should ping `oldest` and call
    /// `replace` only if it fails to answer; otherwise the new peer is dropped.
    Full {
        oldest: Peer,
    },
    /// The peer has the local node's ID and is never stored.
    LocalNode,
}

/// Peers ordered from least to most recently seen, at most `K` of them.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct KBucket {
    peers: Vec<Peer>,
}

impl KBucket {
    pub fn new() -> Self {
        Self { peers: Vec::new() }
    }

    pub fn insert(&mut self, peer: Peer) -> InsertOutcome {
        if let Some(pos) = self.peers.iter().position(|p| p.id == peer.id) {
            self.peers.remove(pos);
            self.peers.push(peer);
            return InsertOutcome::Refreshed;
        }

        if self.peers.len() >= K {
            return InsertOutcome::Full {
                oldest: self.peers[0].clone(),
            };
        }

        self.peers.push(peer);
        InsertOutcome::Inserted
    }

    /// Evicts `stale` (normally the `oldest` from `InsertOutcome::Full` after
    /// it failed a ping) and inserts `peer`. Returns false and changes
    /// nothing if `stale` is no longer in the bucket.
    pub fn replace(&mut self, stale: &NodeId, peer: Peer) -> bool {
        if self.remove(stale).is_none() {
            return false;
        }

        self.insert(peer);
        true
    }

    pub fn remove(&mut self, id: &NodeId) -> Option<Peer> {
        let pos = self.peers.iter().position(|p| &p.id == id)?;
        Some(self.peers.remove(pos))
    }

    pub fn contains(&self, id: &NodeId) -> bool {
        self.peers.iter().any(|p| &p.id == id)
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    pub fn peers(&self) -> &[Peer] {
        &self.peers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(n: usize) -> Peer {
        Peer::new(format!("wolf-{n}"), format!("127.0.0.1:{}", 10000 + n))
    }

    #[test]
    fn insert_ignores_duplicates() {
        let peer = Peer::new("wolf-a", "127.0.0.1:9560");
        let id = peer.id.clone();

        let mut bucket = KBucket::new();
        bucket.insert(peer.clone());
        bucket.insert(peer);

        assert_eq!(bucket.len(), 1);
        assert!(bucket.contains(&id));
    }

    #[test]
    fn remove_peer() {
        let peer = Peer::new("wolf-a", "127.0.0.1:9560");
        let id = peer.id.clone();

        let mut bucket = KBucket::new();
        bucket.insert(peer);

        let removed = bucket.remove(&id);

        assert!(removed.is_some());
        assert!(bucket.is_empty());
    }

    #[test]
    fn duplicate_insert_moves_peer_to_most_recent() {
        let mut bucket = KBucket::new();
        bucket.insert(peer(1));
        bucket.insert(peer(2));

        assert!(matches!(bucket.insert(peer(1)), InsertOutcome::Refreshed));
        assert_eq!(bucket.peers()[1].id, peer(1).id);
        assert_eq!(bucket.len(), 2);
    }

    #[test]
    fn full_bucket_reports_oldest_and_keeps_capacity() {
        let mut bucket = KBucket::new();
        for n in 0..K {
            assert!(matches!(bucket.insert(peer(n)), InsertOutcome::Inserted));
        }

        let outcome = bucket.insert(peer(K));

        assert_eq!(bucket.len(), K);
        assert!(!bucket.contains(&peer(K).id));
        match outcome {
            InsertOutcome::Full { oldest } => assert_eq!(oldest.id, peer(0).id),
            other => panic!("expected Full, got {other:?}"),
        }
    }

    #[test]
    fn replace_evicts_stale_peer_only_if_present() {
        let mut bucket = KBucket::new();
        for n in 0..K {
            bucket.insert(peer(n));
        }

        assert!(!bucket.replace(&peer(99).id, peer(K)));
        assert!(!bucket.contains(&peer(K).id));

        assert!(bucket.replace(&peer(0).id, peer(K)));
        assert_eq!(bucket.len(), K);
        assert!(!bucket.contains(&peer(0).id));
        assert_eq!(bucket.peers()[K - 1].id, peer(K).id);
    }
}
