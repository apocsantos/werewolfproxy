#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Full Gate"
echo "===================="
echo

echo "🎯 Preparing targets"
./scripts/test-targets.sh

echo
echo "🔧 Healing"
wolf-b heal --quiet || true

echo
echo "⚡ Quick gate"
./scripts/quick-gate.sh

echo
echo "📦 JSON contract gate"
./scripts/json-contract-gate.sh
echo
echo "📊 Dashboard gate"
./scripts/dashboard-gate.sh
echo
echo "🐾 Peer gate"
./scripts/peer-gate.sh
echo
echo "🦀 Peer Rust gate"
./scripts/peer-rust-gate.sh




echo
echo "🚪 RC gate"
./scripts/rc-gate.sh

echo
echo "🔥 Chaos gate"
./scripts/chaos-gate.sh

echo
echo "🔥 Extended chaos gate"
./scripts/chaos-gate-extended.sh
echo
echo "🌕 Soak gate with chaos"
./scripts/soak-gate.sh 3 2 --chaos


echo
echo "📸 Snapshot"
wolf-b snapshot >/dev/null

echo
echo "🩺 Final doctor"
wolf-b doctor

echo
echo "🎉 FULL GATE PASSED"
