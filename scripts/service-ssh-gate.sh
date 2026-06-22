#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Service SSH Gate"
echo "==========================="
echo

local_port="${1:-12222}"

if ! timeout 2 bash -lc 'exec 3<>/dev/tcp/127.0.0.1/22' 2>/dev/null; then
  echo "⚠ local SSH server not reachable on 127.0.0.1:22"
  echo "Skipping SSH gate."
  exit 0
fi

./scripts/start-multiwolf.sh >/dev/null
./scripts/peer-db.sh seed-local >/dev/null

./scripts/service-db.sh add gate-ssh wolf-a 127.0.0.1:22 ssh >/dev/null
./scripts/service-db.sh connect gate-ssh "$local_port" >/tmp/wolf-service-ssh-connect.txt

sleep 2

banner="$(timeout 5 bash -lc "exec 3<>/dev/tcp/127.0.0.1/$local_port; head -n 1 <&3" || true)"

if [[ "$banner" == SSH-* ]]; then
  echo "✅ SSH banner over Werewolf received"
  echo "$banner"
else
  echo "❌ SSH banner not received"
  echo "banner: $banner"
  echo
  echo "connect log:"
  cat /tmp/wolf-service-ssh-connect.txt || true
  exit 1
fi

echo
echo "🎉 SERVICE SSH GATE PASSED"
