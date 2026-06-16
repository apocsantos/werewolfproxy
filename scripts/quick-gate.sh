#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Quick Gate"
echo "====================="
echo

echo "🧱 Build"
cargo build

echo
echo "📂 Repo gate"
./scripts/repo-gate.sh
echo
echo "📜 Script gate"
./scripts/script-gate.sh

echo
echo "🔧 Heal"
wolf-b heal --quiet || true

echo
echo "🩺 Doctor"
wolf-b doctor

echo
echo "🧠 Policy test"
wolf-b policy-test

echo
echo "📦 JSON contracts"
wolf-b ready --json | jq -e '.ready == true' >/dev/null
echo "✅ ready JSON"

wolf-b selftest --json | jq -e '.ok == true' >/dev/null
echo "✅ selftest JSON"

wolf-b status-json | jq -e '.ready.ready == true' >/dev/null
echo "✅ status JSON"

echo
echo "🎉 QUICK GATE PASSED"
