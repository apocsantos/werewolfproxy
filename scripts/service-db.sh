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


  plan-connect)
    init_db
    name="${2:?service name required}"
    local_port="${3:-}"

    route="$("$0" route "$name")"

    if ! echo "$route" | jq -e '.available == true' >/dev/null; then
      echo "$route" | jq .
      exit 1
    fi

    kind="$(echo "$route" | jq -r '.kind')"
    target="$(echo "$route" | jq -r '.target')"
    peer="$(echo "$route" | jq -r '.peer')"

    if [[ -z "$local_port" ]]; then
      case "$kind" in
        ssh) local_port="2222" ;;
        http) local_port="8088" ;;
        https) local_port="8448" ;;
        *) local_port="9000" ;;
      esac
    fi

    jq -n \
      --arg service "$name" \
      --arg peer "$peer" \
      --arg target "$target" \
      --arg kind "$kind" \
      --argjson local_port "$local_port" \
      '{
        service: $service,
        peer: $peer,
        target: $target,
        kind: $kind,
        local_bind: ("127.0.0.1:" + ($local_port|tostring)),
        suggested_client:
          (if $kind == "ssh" then
            ("ssh -p " + ($local_port|tostring) + " user@127.0.0.1")
           elif $kind == "http" then
            ("http://127.0.0.1:" + ($local_port|tostring))
           elif $kind == "https" then
            ("https://127.0.0.1:" + ($local_port|tostring))
           else
            ("127.0.0.1:" + ($local_port|tostring))
           end)
      }'
    ;;


  connect-dry-run)
    init_db
    name="${2:?service name required}"
    local_port="${3:-}"

    plan="$("$0" plan-connect "$name" "$local_port")"

    peer="$(echo "$plan" | jq -r '.peer')"
    target="$(echo "$plan" | jq -r '.target')"
    local_bind="$(echo "$plan" | jq -r '.local_bind')"
    kind="$(echo "$plan" | jq -r '.kind')"

    jq -n \
      --arg service "$name" \
      --arg peer "$peer" \
      --arg target "$target" \
      --arg local_bind "$local_bind" \
      --arg kind "$kind" \
      '{
        dry_run: true,
        service: $service,
        peer: $peer,
        target: $target,
        local_bind: $local_bind,
        kind: $kind,
        next_step: "create Werewolf Fang/tunnel for this local_bind -> peer -> target"
      }'
    ;;


  connect)
    init_db
    name="${2:?service name required}"
    local_port="${3:-}"

    plan="$("$0" plan-connect "$name" "$local_port")"

    peer="$(echo "$plan" | jq -r '.peer')"
    target="$(echo "$plan" | jq -r '.target')"
    local_bind="$(echo "$plan" | jq -r '.local_bind')"
    kind="$(echo "$plan" | jq -r '.kind')"

    local_host="${local_bind%:*}"
    local_port_resolved="${local_bind##*:}"

    echo "🐺 Werewolf Service Connect"
    echo "=========================="
    echo "service:     $name"
    echo "kind:        $kind"
    echo "peer:        $peer"
    echo "local bind:  $local_bind"
    echo "target:      $target"
    echo

    fang_output="$(wolf-b fang open "$peer" "$local_host:$local_port_resolved" "$target" 2>&1 || true)"
    echo "$fang_output"

    if echo "$fang_output" | jq -e '.code? // empty' >/dev/null 2>&1; then
      echo
      echo "❌ service connect failed"
      exit 1
    fi

    jq \
      --arg name "$name" \
      --arg ts "$(date -Iseconds)" \
      '.[$name].last_connected = $ts' "$SERVICE_DB" > "$SERVICE_DB.tmp"
    mv "$SERVICE_DB.tmp" "$SERVICE_DB"

    echo
    echo "✅ service connected"
    echo "$plan" | jq .
    ;;


  status)
    init_db
    name="${2:-}"

    if [[ -n "$name" ]]; then
      "$0" show "$name"
      echo
      "$0" route "$name"
      echo
      "$0" plan-connect "$name"
      exit 0
    fi

    echo "🐺 Werewolf Service Status"
    echo "========================="
    echo

    "$0" list
    ;;

  *)
    cat <<HELP
🐺 Werewolf Service DB

Usage:
  scripts/service-db.sh add <name> <peer> <target-host:port> [kind]
  scripts/service-db.sh list
  scripts/service-db.sh show <name>
  scripts/service-db.sh status [name]
  scripts/service-db.sh route <name>
  scripts/service-db.sh plan-connect <name> [local-port]
  scripts/service-db.sh connect-dry-run <name> [local-port]
  scripts/service-db.sh connect <name> [local-port]
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
