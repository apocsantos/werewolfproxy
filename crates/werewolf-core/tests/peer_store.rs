use werewolf_core::nodeid::NodeId;
use werewolf_core::peer::Peer;
use werewolf_core::peer_store::PeerStore;
use werewolf_core::routing::RoutingTable;

#[test]
fn peer_store_roundtrip_preserves_peers() {
    let local = NodeId::from_name_address("wolf-b", "127.0.0.1:9561");

    let wolf_c = Peer::new("wolf-c", "127.0.0.1:9562").with_health(true, 96.0, 100.0, Some(8.0));

    let mut table = RoutingTable::new(local);
    table.add_peer(wolf_c);

    let store = PeerStore::from_routing_table(&table);

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("peers.json");

    store.save(&path).expect("save");

    let loaded = PeerStore::load(&path).expect("load");
    let loaded_table = loaded.into_routing_table();

    assert_eq!(loaded_table.peer_count(), 1);
}
