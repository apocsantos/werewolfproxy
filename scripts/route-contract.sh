#!/usr/bin/env bash
set -euo pipefail

candidate="$(./scripts/peer-db.sh route-candidate)"

jq -n \
  --arg generated_at "$(date -Iseconds)" \
  --argjson candidate "$candidate" \
  '{
    generated_at: $generated_at,
    route_candidate: $candidate
  }'
