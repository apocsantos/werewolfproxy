use werewolf_core::nodeid::NodeId;
use werewolf_core::peer::Peer;
use werewolf_core::routing::RoutingTable;

#[test]
fn local_pack_routing_table_tracks_healthy_peers() {
    let local = NodeId::from_name_address("wolf-b", "127.0.0.1:9561");

    let wolf_a = Peer::new("wolf-a", "127.0.0.1:9560").with_health(true, 95.0, 100.0, Some(10.0));
    let wolf_c = Peer::new("wolf-c", "127.0.0.1:9562").with_health(true, 96.0, 100.0, Some(8.0));
    let wolf_d = Peer::new("wolf-d", "127.0.0.1:9563").with_health(false, 10.0, 30.0, None);

    let mut table = RoutingTable::new(local);
    table.add_peer(wolf_a);
    table.add_peer(wolf_c);
    table.add_peer(wolf_d);

    assert_eq!(table.peer_count(), 3);
    assert_eq!(table.healthy_peers().len(), 2);

    let target = NodeId::from_name_address("target-key", "werewolf");
    let nearest = table.nearest(&target, 2);

    assert_eq!(nearest.len(), 2);
}
