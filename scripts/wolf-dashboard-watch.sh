#!/usr/bin/env bash
set -euo pipefail

interval="${1:-5}"

while true; do
  ./scripts/wolf-dashboard.sh
  echo
  echo "refresh: ${interval}s | Ctrl+C to stop"
  sleep "$interval"
done
