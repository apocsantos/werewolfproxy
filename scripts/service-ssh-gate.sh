#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Service SSH Gate"
echo "==========================="
echo

port="${1:-12222}"

if ! timeout 2 bash -lc 'cat < /dev/null > /dev/tcp/127.0.0.1/22' 2>/dev/null; then
  echo "⚠ local SSH server not reachable on 127.0.0.1:22"
  echo "Skipping SSH gate."
  exit 0
fi

./scripts/start-multiwolf.sh >/dev/null
./scripts/peer-db.sh seed-local >/dev/null

./scripts/service-db.sh add gate-ssh wolf-a 127.0.0.1:22 ssh >/dev/null
./scripts/service-db.sh connect gate-ssh "$port" >/tmp/wolf-service-ssh-connect.txt

sleep 2

if timeout 5 bash -lc "cat < /dev/null > /dev/tcp/127.0.0.1/$port" 2>/dev/null; then
  echo "✅ SSH TCP service over Werewolf reachable"
else
  echo "❌ SSH TCP service over Werewolf not reachable"
  exit 1
fi

echo
echo "🎉 SERVICE SSH GATE PASSED"
