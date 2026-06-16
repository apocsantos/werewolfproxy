#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Recovery"
echo "===================="
echo

echo "📦 Runtime dump"
./scripts/runtime-dump.sh || true

echo "📂 Project"
cd "$(dirname "$0")/.."

echo
echo "🔄 Restart services"
./scripts/systemd-restart.sh || true

echo
echo "🎯 Recreate targets"
./scripts/test-targets.sh || true

echo
echo "🔧 Heal transports"
wolf-b heal || true

echo
echo "🧪 Selftest"
if wolf-b selftest --json | jq -e '.ok == true' >/dev/null; then
  echo "✅ selftest healthy"
else
  echo "⚠ selftest degraded"
fi

echo
echo "📊 Final status"
wolf-b status-plus || true

echo
echo "🩺 Final doctor"
wolf-b doctor || true

echo
echo "🎉 Recovery complete"
