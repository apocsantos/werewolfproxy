use std::env;
use std::path::PathBuf;

use werewolf_core::nodeid::NodeId;
use werewolf_core::peer::Peer;
use werewolf_core::peer_store::PeerStore;
use werewolf_core::routing::RoutingTable;

fn sample_table() -> RoutingTable {
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

    table
}

fn print_usage() {
    eprintln!(
        "Usage:
  peer_lab store
  peer_lab nearest <name> <address> [limit]
  peer_lab save <path>
  peer_lab load <path>
  peer_lab nearest-from <path> <name> <address> [limit]"
    );
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(cmd) = args.next() else {
        print_usage();
        std::process::exit(2);
    };

    match cmd.as_str() {
        "store" => {
            let table = sample_table();
            let store = PeerStore::from_routing_table(&table);
            println!(
                "{}",
                serde_json::to_string_pretty(&store).expect("serialize peer store")
            );
        }

        "nearest" => {
            let Some(name) = args.next() else {
                print_usage();
                std::process::exit(2);
            };

            let Some(address) = args.next() else {
                print_usage();
                std::process::exit(2);
            };

            let limit = args
                .next()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(3);

            let table = sample_table();
            let target = NodeId::from_name_address(&name, &address);
            let nearest = table.nearest(&target, limit);

            println!(
                "{}",
                serde_json::to_string_pretty(&nearest).expect("serialize nearest")
            );
        }

        "save" => {
            let Some(path) = args.next() else {
                print_usage();
                std::process::exit(2);
            };

            let table = sample_table();
            let store = PeerStore::from_routing_table(&table);
            let path = PathBuf::from(path);

            store.save(&path).expect("save peer store");

            println!(
                "{{\"saved\":true,\"path\":\"{}\",\"peer_count\":{}}}",
                path.display(),
                store.peers.len()
            );
        }

        "load" => {
            let Some(path) = args.next() else {
                print_usage();
                std::process::exit(2);
            };

            let store = PeerStore::load(PathBuf::from(path)).expect("load peer store");

            println!(
                "{}",
                serde_json::to_string_pretty(&store).expect("serialize loaded store")
            );
        }

        "nearest-from" => {
            let Some(path) = args.next() else {
                print_usage();
                std::process::exit(2);
            };

            let Some(name) = args.next() else {
                print_usage();
                std::process::exit(2);
            };

            let Some(address) = args.next() else {
                print_usage();
                std::process::exit(2);
            };

            let limit = args
                .next()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(3);

            let store = PeerStore::load(PathBuf::from(path)).expect("load peer store");
            let table = store.into_routing_table();
            let target = NodeId::from_name_address(&name, &address);
            let nearest = table.nearest(&target, limit);

            println!(
                "{}",
                serde_json::to_string_pretty(&nearest).expect("serialize nearest")
            );
        }

        _ => {
            print_usage();
            std::process::exit(2);
        }
    }
}
