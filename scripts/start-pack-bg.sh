#!/usr/bin/env bash
: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required for local control}"
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"


if systemctl --user is-active --quiet werewolf-a || systemctl --user is-active --quiet werewolf-b; then
  echo "❌ systemd Werewolf services are active."
  echo "Stop them first:"
  echo "  systemctl --user stop werewolf-a werewolf-b"
  exit 1
fi


mkdir -p logs

pkill -f "werewolfd.*wolf-a" || true
pkill -f "werewolfd.*wolf-b" || true


echo "🐺 starting wolf-a..."
nohup ./scripts/start-wolf-a.sh > logs/wolf-a.log 2>&1 &

sleep 2

echo "🐺 starting wolf-b..."
nohup ./scripts/start-wolf-b.sh > logs/wolf-b.log 2>&1 &

sleep 3

echo "sockets:"
ls -l "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-a/control.sock" "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-b/control.sock"

echo "✅ Pack started in background"
echo "logs/wolf-a.log"
echo "logs/wolf-b.log"
