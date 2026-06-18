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
        score: 0
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
       | .[$name].score = $score' "$PEER_DB" > "$PEER_DB.tmp"
    mv "$PEER_DB.tmp" "$PEER_DB"

    jq --arg name "$name" '.[$name]' "$PEER_DB"
    ;;

  *)
    cat <<HELP
🐺 Werewolf Peer DB

Usage:
  scripts/peer-db.sh add <name> <host:port>
  scripts/peer-db.sh list
  scripts/peer-db.sh ping <name>
  scripts/peer-db.sh remove <name>

DB:
  $PEER_DB
HELP
    ;;
esac
