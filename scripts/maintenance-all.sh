#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Maintenance All"
echo "=========================="
echo

echo "📦 Runtime dump"
./scripts/runtime-dump.sh || true

echo
echo "🧹 Wolf maintenance"
wolf-b maintenance || true

echo
echo "🗑 Dump prune"
./scripts/dump-prune.sh 20 || true

echo
echo "📸 Snapshot"
wolf-b snapshot >/dev/null || true

echo
echo "🩺 Final doctor"
wolf-b doctor || true

echo
echo "📊 Cache status"
wolf-b cache-status || true

echo
echo "🎉 Maintenance complete"
