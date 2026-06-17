#!/usr/bin/env bash
set -euo pipefail

iterations="${1:-30}"
delay="${2:-10}"

echo "🐺 Werewolf Nightly Gate"
echo "======================="
echo "iterations: $iterations"
echo "delay:      ${delay}s"
echo

echo "🎯 Preparing targets"
./scripts/test-targets.sh

echo
echo "🔧 Initial heal"
wolf-b heal --quiet || true

echo
echo "🚪 Release gate"
./scripts/release-gate.sh

echo
echo "🔥 Extended chaos"
./scripts/chaos-gate-extended.sh

echo
echo "🌕 Long chaos soak"
./scripts/soak-gate.sh "$iterations" "$delay" --chaos

echo
echo "📸 Final snapshot"
wolf-b snapshot >/dev/null

echo
echo "🩺 Final doctor"
wolf-b doctor

echo
echo "🎉 NIGHTLY GATE PASSED"
