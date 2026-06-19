#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Contract Update"
echo "==========================="
echo

mkdir -p contracts

echo "🔧 Healing before capture"
wolf-b heal --quiet || true

echo "📦 Capturing JSON contracts"

wolf-b doctor --json > contracts/doctor.json
wolf-b ready --json > contracts/ready.json
wolf-b selftest --json > contracts/selftest.json
wolf-b status-json > contracts/status.json
wolf-b cache-status --json > contracts/cache-status.json
wolf-b benchmark --json > contracts/benchmark.json
./scripts/wolf-dashboard-json.sh > contracts/dashboard.json
./scripts/peer-contract.sh > contracts/peer.json

echo
echo "🧬 Validating schemas"
./scripts/schema-gate.sh

echo
echo "✅ contracts updated"
