use werewolf_core::nodeid::NodeId;
use werewolf_core::peer::Peer;
use werewolf_core::peer_store::PeerStore;
use werewolf_core::routing::RoutingTable;

fn main() {
    let local = NodeId::from_name_address("wolf-b", "127.0.0.1:9561");

    let peers = vec![
        Peer::new("wolf-a", "127.0.0.1:9560").with_health(true, 95.0, 100.0, Some(10.0)),
        Peer::new("wolf-c", "127.0.0.1:9562").with_health(true, 96.0, 100.0, Some(8.0)),
        Peer::new("wolf-d", "127.0.0.1:9563").with_health(true, 94.0, 100.0, Some(12.0)),
    ];

    let mut table = RoutingTable::new(local);

    for peer in peers {
        table.add_peer(peer);
    }

    let store = PeerStore::from_routing_table(&table);

    println!("{}", serde_json::to_string_pretty(&store).expect("serialize peer store"));
}
