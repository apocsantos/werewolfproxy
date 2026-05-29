#!/usr/bin/env bash
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"


if systemctl --user is-active --quiet werewolf-a || systemctl --user is-active --quiet werewolf-b; then
  echo "❌ systemd Werewolf services are active."
  echo "Stop them first:"
  echo "  systemctl --user stop werewolf-a werewolf-b"
  exit 1
fi


pkill -f "werewolfd.*wolf-a" || true
pkill -f "werewolfd.*wolf-b" || true

rm -f /tmp/wolf-a.sock /tmp/wolf-b.sock

echo "🐺 starting wolf-a..."
konsole --workdir "$ROOT" -e bash -c "./scripts/start-wolf-a.sh; exec bash" &

sleep 2

echo "🐺 starting wolf-b..."
konsole --workdir "$ROOT" -e bash -c "./scripts/start-wolf-b.sh; exec bash" &

sleep 3

echo "✅ Pack started"
