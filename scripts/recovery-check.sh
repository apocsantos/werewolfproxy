#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Recovery Check"
echo "========================="
echo

./scripts/recover-wolf.sh

echo
echo "🧪 Quick gate after recovery"
./scripts/quick-gate.sh

echo
echo "🔥 Basic chaos after recovery"
./scripts/chaos-gate.sh

echo
echo "🎉 RECOVERY CHECK PASSED"
