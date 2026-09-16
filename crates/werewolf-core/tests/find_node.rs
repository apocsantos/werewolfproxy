use werewolf_core::find_node::handle_find_node;
use werewolf_core::message::Message;
use werewolf_core::nodeid::NodeId;
use werewolf_core::peer::Peer;
use werewolf_core::routing::RoutingTable;

#[test]
fn handle_find_node_returns_nearest_peers() {
    let local = NodeId::from_name_address("wolf-b", "127.0.0.1:9561");
    let sender = NodeId::from_name_address("wolf-a", "127.0.0.1:9560");
    let target = NodeId::from_name_address("resource", "hash");

    let mut table = RoutingTable::new(local.clone());

    table.add_peer(Peer::new("wolf-c", "127.0.0.1:9562").with_health(true, 96.0, 100.0, Some(8.0)));

    table.add_peer(Peer::new("wolf-d", "127.0.0.1:9563").with_health(
        true,
        94.0,
        100.0,
        Some(12.0),
    ));

    let request = Message::FindNode {
        sender,
        target,
        k: 1,
    };

    let response = handle_find_node(&local, &table, &request).expect("response");

    match response {
        Message::FindNodeResult { peers, .. } => {
            assert_eq!(peers.len(), 1);
        }
        _ => panic!("wrong response type"),
    }
}

#[test]
fn handle_find_node_ignores_non_find_node_messages() {
    let local = NodeId::from_name_address("wolf-b", "127.0.0.1:9561");
    let table = RoutingTable::new(local.clone());

    let request = Message::Ping {
        sender: NodeId::from_name_address("wolf-a", "127.0.0.1:9560"),
    };

    assert!(handle_find_node(&local, &table, &request).is_none());
}
