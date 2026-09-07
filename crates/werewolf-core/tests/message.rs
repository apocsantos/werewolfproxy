use werewolf_core::message::Message;
use werewolf_core::nodeid::NodeId;
use werewolf_core::peer::Peer;

#[test]
fn find_node_message_serializes() {
    let sender = NodeId::from_name_address("wolf-a", "127.0.0.1:9560");
    let target = NodeId::from_name_address("resource", "hash");

    let msg = Message::FindNode {
        sender,
        target,
        k: 3,
    };

    let json = serde_json::to_string(&msg).expect("serialize");
    let decoded: Message = serde_json::from_str(&json).expect("deserialize");

    match decoded {
        Message::FindNode { k, .. } => assert_eq!(k, 3),
        _ => panic!("wrong message type"),
    }
}

#[test]
fn find_node_result_message_serializes_peers() {
    let sender = NodeId::from_name_address("wolf-b", "127.0.0.1:9561");
    let target = NodeId::from_name_address("resource", "hash");

    let peers =
        vec![Peer::new("wolf-c", "127.0.0.1:9562").with_health(true, 96.0, 100.0, Some(8.0))];

    let msg = Message::FindNodeResult {
        sender,
        target,
        peers,
    };

    let json = serde_json::to_string(&msg).expect("serialize");
    let decoded: Message = serde_json::from_str(&json).expect("deserialize");

    match decoded {
        Message::FindNodeResult { peers, .. } => assert_eq!(peers.len(), 1),
        _ => panic!("wrong message type"),
    }
}
