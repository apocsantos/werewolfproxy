#!/usr/bin/env bash
set -euo pipefail

name="${1:?usage: tunnel-health.sh <name> <url> [timeout]}"
url="${2:?usage: tunnel-health.sh <name> <url> [timeout]}"
timeout="${3:-5}"

if curl --max-time "$timeout" -fsS "$url" >/dev/null; then
  echo "$name healthy"
  exit 0
else
  echo "$name failed"
  exit 1
fi
