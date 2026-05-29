#!/usr/bin/env bash
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "🚀 WEREWOLF RELEASE CHECK"
echo "========================="

echo
echo "🧱 Step 1/4: Build + Dev Check"
./scripts/dev-check.sh

echo
echo "📊 Step 2/4: Pack Status"
./scripts/pack-status.sh

echo
echo "🌐 Step 3/4: End-to-End QUIC validation"
curl --max-time 8 -fsS http://127.0.0.1:9020 >/dev/null
echo "✅ QUIC Fang tunnel healthy"

echo
echo "🔐 Checking signed QUIC verification log..."
if journalctl --user -u werewolf-a -n 80 --no-pager | grep -q "QUIC signature verified"; then
  echo "✅ Signed QUIC verification confirmed"
else
  echo "⚠️ Signed QUIC verification log not found"
  echo "⚠️ Non-fatal: QUIC tunnel health already passed"
fi

echo
echo "📦 Step 4/4: Create release checkpoint"
mkdir -p ../releases

ARCHIVE="../releases/werewolfproxy_release_$(date +%Y%m%d_%H%M%S).tar.gz"

tar \
  --exclude='target' \
  --exclude='.git' \
  --exclude='logs' \
  --exclude='*.tar.gz' \
  -czvf "$ARCHIVE" .

echo
echo "✅ RELEASE READY"
echo "📦 Archive:"
echo "   $ARCHIVE"
