#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Chaos Gate"
echo "====================="
echo

./scripts/test-targets.sh
wolf-b heal --quiet

./scripts/transport-chaos.sh

echo
echo "🧪 Final RC sanity"
./scripts/rc-gate.sh

echo
echo "🎉 CHAOS GATE PASSED"
