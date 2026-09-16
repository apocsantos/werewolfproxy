#!/usr/bin/env bash
set -euo pipefail

jq -n \
  --arg timestamp "$(date -Iseconds)" \
  --argjson doctor "$(wolf-b doctor --json)" \
  --argjson ready "$(wolf-b ready --json)" \
  --argjson status "$(wolf-b status-json)" \
  --argjson scores "$(wolf-b score-json)" \
  --argjson cache "$(wolf-b cache-status --json)" \
  '{
    timestamp: $timestamp,
    doctor: $doctor,
    ready: $ready,
    status: $status,
    scores: $scores,
    cache: $cache
  }'
