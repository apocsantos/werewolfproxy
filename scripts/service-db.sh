#!/usr/bin/env bash
set -euo pipefail

SERVICE_DB="${WEREWOLF_SERVICE_DB:-$HOME/.config/wolf-b/services.json}"
mkdir -p "$(dirname "$SERVICE_DB")"

init_db() {
  [[ -f "$SERVICE_DB" ]] || echo '{}' > "$SERVICE_DB"
}

case "${1:-}" in
  add)
    init_db
    name="${2:?service name required}"
    peer="${3:?peer name required}"
    target="${4:?target host:port required}"
    kind="${5:-tcp}"

    jq \
      --arg name "$name" \
      --arg peer "$peer" \
      --arg target "$target" \
      --arg kind "$kind" \
      --arg ts "$(date -Iseconds)" \
      '.[$name] = {
        peer: $peer,
        target: $target,
        kind: $kind,
        added_at: $ts,
        last_connected: null
      }' "$SERVICE_DB" > "$SERVICE_DB.tmp"

    mv "$SERVICE_DB.tmp" "$SERVICE_DB"
    echo "✅ service added: $name -> $peer/$target ($kind)"
    ;;

  list)
    init_db
    jq . "$SERVICE_DB"
    ;;

  remove)
    init_db
    name="${2:?service name required}"

    jq --arg name "$name" 'del(.[$name])' "$SERVICE_DB" > "$SERVICE_DB.tmp"
    mv "$SERVICE_DB.tmp" "$SERVICE_DB"

    echo "✅ service removed: $name"
    ;;

  show)
    init_db
    name="${2:?service name required}"
    jq --arg name "$name" '.[$name]' "$SERVICE_DB"
    ;;

  route)
    init_db
    name="${2:?service name required}"

    service="$(jq -r --arg name "$name" '.[$name] // empty' "$SERVICE_DB")"

    if [[ -z "$service" || "$service" == "null" ]]; then
      echo "❌ unknown service: $name"
      exit 1
    fi

    peer="$(jq -r --arg name "$name" '.[$name].peer' "$SERVICE_DB")"
    target="$(jq -r --arg name "$name" '.[$name].target' "$SERVICE_DB")"
    kind="$(jq -r --arg name "$name" '.[$name].kind' "$SERVICE_DB")"

    peer_info="$(./scripts/peer-db.sh export | jq --arg peer "$peer" '.peers[] | select(.name == $peer)' || true)"

    if [[ -z "$peer_info" ]]; then
      jq -n \
        --arg service "$name" \
        --arg peer "$peer" \
        --arg target "$target" \
        --arg kind "$kind" \
        '{available:false, service:$service, peer:$peer, target:$target, kind:$kind, reason:"peer not found"}'
      exit 0
    fi

    healthy="$(echo "$peer_info" | jq -r '.healthy')"
    reputation="$(echo "$peer_info" | jq -r '.reputation')"

    jq -n \
      --arg service "$name" \
      --arg peer "$peer" \
      --arg target "$target" \
      --arg kind "$kind" \
      --argjson healthy "$healthy" \
      --argjson reputation "$reputation" \
      '{
        available: $healthy,
        service: $service,
        peer: $peer,
        target: $target,
        kind: $kind,
        peer_reputation: $reputation
      }'
    ;;

  seed-local)
    init_db
    "$0" add wolf-c-ssh wolf-c 127.0.0.1:22 ssh
    "$0" add wolf-c-http wolf-c 127.0.0.1:80 http
    "$0" add wolf-d-ssh wolf-d 127.0.0.1:22 ssh
    ;;

  *)
    cat <<HELP
🐺 Werewolf Service DB

Usage:
  scripts/service-db.sh add <name> <peer> <target-host:port> [kind]
  scripts/service-db.sh list
  scripts/service-db.sh show <name>
  scripts/service-db.sh route <name>
  scripts/service-db.sh seed-local
  scripts/service-db.sh remove <name>

Examples:
  scripts/service-db.sh add home-ssh wolf-c 192.168.1.10:22 ssh
  scripts/service-db.sh add home-http wolf-c 192.168.1.20:80 http

DB:
  $SERVICE_DB
HELP
    ;;
esac
