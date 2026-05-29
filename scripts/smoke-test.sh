#!/usr/bin/env bash
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "🐺 Wolf A status:"
./scripts/wolf-a.sh status

echo
echo "🐺 Wolf B status before:"
./scripts/wolf-b.sh status

echo
echo "🦷 Closing old Fang if present..."
./scripts/wolf-b.sh fang close fang_1 >/dev/null 2>&1 || true

echo "🦷 Opening QUIC Fang profile..."
./scripts/wolf-b.sh fang open-profile home-web-quic

echo
echo "🌐 Testing HTTP through QUIC Fang..."
curl --max-time 8 -fsS http://127.0.0.1:9020 >/dev/null

echo
echo "🐺 Wolf B status after:"
./scripts/wolf-b.sh status

echo
echo "✅ Smoke test passed"
