#!/usr/bin/env bash
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "🐺 Wolf A"
if ./scripts/wolf-a.sh status; then
  echo "✅ wolf-a online"
else
  echo "❌ wolf-a offline"
fi

echo
echo "🐺 Wolf B"
if ./scripts/wolf-b.sh status; then
  echo "✅ wolf-b online"
else
  echo "❌ wolf-b offline"
fi

echo
echo "🐾 Wolf B pack"
./scripts/wolf-b.sh pack list || true

echo
echo "🧹 Wolf B fang cleanup"
./scripts/wolf-b.sh fang cleanup || true

echo
echo "🦷 Wolf B fangs"
./scripts/wolf-b.sh fang list || true

echo
echo "🌐 Local TCP ports"
ss -ltnp | grep -E '8443|9443|9020' || echo "No Werewolf TCP ports listening"

echo
echo "⚡ Local QUIC/UDP ports"
ss -lunp | grep -E '9560|9561' || echo "No Werewolf QUIC UDP ports listening"
