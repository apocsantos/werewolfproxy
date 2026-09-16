#!/usr/bin/env bash
set -euo pipefail

interval="${1:-30}"

echo "🐺 Werewolf Peer Monitor"
echo "======================="
echo "interval: ${interval}s"
echo

while true; do
  ./scripts/peer-db.sh ping-all >/dev/null 2>&1 || true

  echo
  echo "timestamp: $(date -Iseconds)"
  ./scripts/peer-db.sh best
  echo

  sleep "$interval"
done
