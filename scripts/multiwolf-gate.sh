#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Multiwolf Gate"
echo "========================="
echo

./scripts/start-multiwolf.sh
sleep 2

echo "🩺 Health"
wolf-a health >/dev/null && echo "✅ wolf-a healthy"
wolf-b health >/dev/null && echo "✅ wolf-b healthy"
wolf-c health >/dev/null && echo "✅ wolf-c healthy"
wolf-d health >/dev/null && echo "✅ wolf-d healthy"

echo
echo "🔗 Linking local pack"
./scripts/link-local-pack.sh

echo
echo "🐾 Pack sizes"
wolf-a pack list
wolf-b pack list
wolf-c pack list
wolf-d pack list

echo
echo "🎉 MULTIWOLF GATE PASSED"
