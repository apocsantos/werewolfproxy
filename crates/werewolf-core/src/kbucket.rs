use crate::nodeid::NodeId;
use crate::peer::Peer;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct KBucket {
    peers: Vec<Peer>,
}

impl KBucket {
    pub fn new() -> Self {
        Self { peers: Vec::new() }
    }

    pub fn insert(&mut self, peer: Peer) {
        if self.contains(&peer.id) {
            return;
        }

        self.peers.push(peer);
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
}
