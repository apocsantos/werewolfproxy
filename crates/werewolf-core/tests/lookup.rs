use werewolf_core::lookup::lookup;
use werewolf_core::nodeid::NodeId;
use werewolf_core::peer::Peer;
use werewolf_core::routing::RoutingTable;

#[test]
fn lookup_returns_closest_peers() {
    let local = NodeId::from_name_address("wolf-b", "127.0.0.1:9561");

    let mut table = RoutingTable::new(local);

    table.add_peer(Peer::new("wolf-a", "127.0.0.1:9560").with_health(
        true,
        95.0,
        100.0,
        Some(10.0),
    ));

    table.add_peer(Peer::new("wolf-c", "127.0.0.1:9562").with_health(true, 96.0, 100.0, Some(8.0)));

    let target = NodeId::from_name_address("resource", "hash");
    let result = lookup(&table, &target, 2);

    assert_eq!(result.requested, 2);
    assert_eq!(result.returned, 2);
    assert_eq!(result.peers.len(), 2);
}

#[test]
fn lookup_respects_limit() {
    let local = NodeId::from_name_address("wolf-b", "127.0.0.1:9561");

    let mut table = RoutingTable::new(local);

    table.add_peer(Peer::new("wolf-a", "127.0.0.1:9560"));
    table.add_peer(Peer::new("wolf-c", "127.0.0.1:9562"));
    table.add_peer(Peer::new("wolf-d", "127.0.0.1:9563"));

    let target = NodeId::from_name_address("resource", "hash");
    let result = lookup(&table, &target, 1);

    assert_eq!(result.requested, 1);
    assert_eq!(result.returned, 1);
    assert_eq!(result.peers.len(), 1);
}
