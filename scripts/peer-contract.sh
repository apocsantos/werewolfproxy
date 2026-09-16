#!/usr/bin/env bash
set -euo pipefail

PEER_DB="${WEREWOLF_PEER_DB:-$HOME/.config/wolf-b/peers.json}"

if [[ ! -f "$PEER_DB" ]]; then
    echo '{"error":"peer database not found"}'
    exit 1
fi

jq '
{
  generated_at: (now | todateiso8601),
  peer_count: (keys | length),
  best_peer:
    (
      to_entries
      | sort_by(.value.reputation // 0)
      | reverse
      | .[0].key
    ),
  peers:
    (
      to_entries
      | sort_by(.value.reputation // 0)
      | reverse
      | map({
          name: .key,
          address: .value.address,
          healthy: .value.healthy,
          reputation: (.value.reputation // 0),
          availability_pct: (.value.availability_pct // 0),
          avg_latency_ms: (.value.avg_latency_ms // null),
          total_pings: (.value.total_pings // 0),
          successful_pings: (.value.successful_pings // 0),
          failed_pings: (.value.failed_pings // 0),
          last_seen: .value.last_seen
      })
    )
}
' "$PEER_DB"
