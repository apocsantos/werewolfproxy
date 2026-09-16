#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Peer Export Gate"
echo "==========================="
echo

tmp="/tmp/werewolf-peer-export.json"

./scripts/peer-db.sh export > "$tmp"

jq -e '.peers | type == "array"' "$tmp" >/dev/null
echo "✅ peers array valid"

jq -e '.generated_at != null' "$tmp" >/dev/null
echo "✅ generated_at present"

echo
jq . "$tmp"

echo
echo "🎉 PEER EXPORT GATE PASSED"
