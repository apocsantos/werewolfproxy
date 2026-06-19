#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Peer Gate"
echo "===================="
echo

./scripts/peer-contract.sh >/tmp/wolf-peer.json

jq -e '.peer_count >= 0' /tmp/wolf-peer.json >/dev/null
echo "✅ peer contract valid"

jq -e '.peers' /tmp/wolf-peer.json >/dev/null
echo "✅ peers array present"

echo
echo "🎉 PEER GATE PASSED"
