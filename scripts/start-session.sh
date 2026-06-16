#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Session Start"
echo "========================"
echo

echo "📂 Project"
cd "$(dirname "$0")/.."

echo
echo "🎯 Targets"
./scripts/test-targets.sh

echo
echo "🔄 Restart services"
./scripts/systemd-restart.sh || true

echo
echo "🔧 Heal"
wolf-b heal --quiet || true

echo
echo "🩺 Doctor"
wolf-b doctor || true

echo
echo "📊 Status"
wolf-b status-plus || true

echo
echo "📦 Cache"
wolf-b cache-status || true

echo
echo "📝 Latest report"
wolf-b report-list 3 || true

echo

echo
echo "📊 Dashboard"
./scripts/wolf-dashboard.sh || true

echo "🌕 Wolf ready for the night"
