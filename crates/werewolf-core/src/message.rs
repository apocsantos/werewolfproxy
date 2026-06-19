use crate::nodeid::NodeId;
use crate::peer::Peer;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Message {
    Ping {
        sender: NodeId,
    },

    Pong {
        sender: NodeId,
    },

    FindNode {
        sender: NodeId,
        target: NodeId,
        k: usize,
    },

    FindNodeResult {
        sender: NodeId,
        target: NodeId,
        peers: Vec<Peer>,
    },
}
