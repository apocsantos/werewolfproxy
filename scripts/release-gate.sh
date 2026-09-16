#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Release Gate"
echo "======================="
echo

echo "🧱 Build"
cargo build

echo
echo "🎯 Targets"
./scripts/test-targets.sh

echo
echo "🔧 Heal"
wolf-b heal --quiet || true

echo
echo "📦 JSON contracts"
./scripts/json-contract-gate.sh

echo
echo "🚪 RC gate"
./scripts/rc-gate.sh

echo
echo "📸 Snapshot"
wolf-b snapshot >/dev/null

echo
echo "🩺 Final ready"
wolf-b ready --json | jq -e '.ready == true' >/dev/null

echo
echo "🎉 RELEASE GATE PASSED"
