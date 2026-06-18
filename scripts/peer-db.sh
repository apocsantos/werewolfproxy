#!/usr/bin/env bash
set -euo pipefail

PEER_DB="${WEREWOLF_PEER_DB:-$HOME/.config/wolf-b/peers.json}"
mkdir -p "$(dirname "$PEER_DB")"

init_db() {
  [[ -f "$PEER_DB" ]] || echo '{}' > "$PEER_DB"
}

case "${1:-}" in
  add)
    init_db
    name="${2:?peer name required}"
    address="${3:?peer address required}"
    jq \
      --arg name "$name" \
      --arg address "$address" \
      --arg ts "$(date -Iseconds)" \
      '.[$name] = {
        address: $address,
        added_at: $ts,
        last_seen: null,
        healthy: null,
        latency_ms: null,
        score: 0,
        successful_pings: 0,
        failed_pings: 0,
        total_pings: 0,
        availability_pct: 0,
        avg_latency_ms: null,
        reputation: 0
      }' "$PEER_DB" > "$PEER_DB.tmp"
    mv "$PEER_DB.tmp" "$PEER_DB"
    echo "✅ peer added: $name -> $address"
    ;;

  list)
    init_db
    jq . "$PEER_DB"
    ;;

  remove)
    init_db
    name="${2:?peer name required}"
    jq --arg name "$name" 'del(.[$name])' "$PEER_DB" > "$PEER_DB.tmp"
    mv "$PEER_DB.tmp" "$PEER_DB"
    echo "✅ peer removed: $name"
    ;;

  ping)
    init_db
    name="${2:?peer name required}"
    address="$(jq -r --arg name "$name" '.[$name].address // empty' "$PEER_DB")"

    if [[ -z "$address" ]]; then
      echo "❌ unknown peer: $name"
      exit 1
    fi

    host="${address%:*}"
    port="${address##*:}"

    start_ns="$(date +%s%N)"
    if timeout 3 bash -lc "cat < /dev/null > /dev/tcp/$host/$port" 2>/dev/null; then
      end_ns="$(date +%s%N)"
      latency_ms="$(( (end_ns - start_ns) / 1000000 ))"
      healthy=true
      score=$((100 - latency_ms))
      [[ "$score" -lt 1 ]] && score=1
    else
      latency_ms=null
      healthy=false
      score=0
    fi

    jq \
      --arg name "$name" \
      --arg ts "$(date -Iseconds)" \
      --argjson healthy "$healthy" \
      --argjson latency "$latency_ms" \
      --argjson score "$score" \
      '.[$name].last_seen = $ts
       | .[$name].healthy = $healthy
       | .[$name].latency_ms = $latency
       | .[$name].score = $score
       | .[$name].total_pings = ((.[$name].total_pings // 0) + 1)
       | .[$name].successful_pings = ((.[$name].successful_pings // 0) + (if $healthy then 1 else 0 end))
       | .[$name].failed_pings = ((.[$name].failed_pings // 0) + (if $healthy then 0 else 1 end))
       | .[$name].availability_pct = ((.[$name].successful_pings / .[$name].total_pings) * 100)
       | .[$name].avg_latency_ms =
          (if $healthy and $latency != null then
             (((.[$name].avg_latency_ms // $latency) + $latency) / 2)
           else
             .[$name].avg_latency_ms
           end)
       | .[$name].reputation =
          (((.[$name].availability_pct // 0) * 0.7)
           + ((.[$name].score // 0) * 0.3))' "$PEER_DB" > "$PEER_DB.tmp"
    mv "$PEER_DB.tmp" "$PEER_DB"

    jq --arg name "$name" '.[$name]' "$PEER_DB"
    ;;
  best)
    init_db

    jq '
      to_entries
      | sort_by(.value.reputation // 0)
      | reverse
      | .[]
      | {
          peer: .key,
          reputation: (.value.reputation // 0),
          availability: (.value.availability_pct // 0),
          latency_ms: (.value.avg_latency_ms // null)
        }
    ' "$PEER_DB"
    ;;

  ping-all)
    init_db

    peers="$(jq -r 'keys[]' "$PEER_DB")"

    if [[ -z "$peers" ]]; then
      echo "⚠ no peers in database"
      exit 0
    fi

    while read -r peer; do
      [[ -z "$peer" ]] && continue
      echo "🐾 pinging $peer"
      "$0" ping "$peer"
      echo
    done <<< "$peers"
    ;;

  *)
    cat <<HELP
🐺 Werewolf Peer DB

Usage:
  scripts/peer-db.sh add <name> <host:port>
  scripts/peer-db.sh list
  scripts/peer-db.sh ping <name>
  scripts/peer-db.sh ping-all
  scripts/peer-db.sh best
  scripts/peer-db.sh remove <name>

DB:
  $PEER_DB
HELP
    ;;
esac
