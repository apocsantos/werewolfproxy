#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Dev Gate"
echo "==================="
echo

echo "🎯 Preparing local targets"
./scripts/test-targets.sh

echo
echo "🔧 Healing transports"
wolf-b heal --quiet || true

echo
echo "🧪 Wolf-B selftest"
wolf-b selftest

echo
echo "🔥 Extended chaos gate"
./scripts/chaos-gate-extended.sh

echo
echo "🚪 RC gate"
./scripts/rc-gate.sh

echo
echo "🎉 DEV GATE PASSED"
