#!/usr/bin/env bash
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "🧱 Building..."
cargo build

echo
echo "🛑 Resetting pack..."
./scripts/stop-pack.sh

echo
echo "🐺 Starting pack..."
./scripts/start-pack-bg.sh

echo
echo "🧪 Running smoke test..."
./scripts/smoke-test.sh

echo
echo "✅ Dev check passed"
