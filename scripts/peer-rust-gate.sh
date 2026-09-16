#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Peer Rust Gate"
echo "========================="
echo

export_json="/tmp/werewolf-peer-export.json"
store_json="/tmp/werewolf-peer-store.json"

echo "🐾 Seed local peers"
./scripts/start-multiwolf.sh >/dev/null
./scripts/peer-db.sh seed-local >/dev/null

echo "📤 Export shell peer DB"
./scripts/peer-db.sh export > "$export_json"
jq . "$export_json" >/dev/null
echo "✅ shell export valid"

echo "🦀 Import into Rust PeerStore"
cargo run -q -p werewolf-core --bin peer_lab -- import-export "$export_json" "$store_json" | jq .
jq . "$store_json" >/dev/null
echo "✅ Rust store valid"

echo "🧭 Query nearest from Rust"
cargo run -q -p werewolf-core --bin peer_lab -- nearest-from "$store_json" target-key werewolf 2 | jq .
echo "✅ Rust nearest valid"

echo
echo "🎉 PEER RUST GATE PASSED"
