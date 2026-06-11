#!/usr/bin/env bash
set -e

SOCKET="/tmp/wolf-b.sock"
SERVICE="werewolf-b"
TARGET_URL="${WEREWOLF_TARGET_URL:-http://127.0.0.1:8080}"
QUIC_URL="${WEREWOLF_QUIC_URL:-http://127.0.0.1:9020}"
TCP_URL="${WEREWOLF_TCP_URL:-http://127.0.0.1:9021}"
TCP_ENC_URL="${WEREWOLF_TCP_ENC_URL:-http://127.0.0.1:9022}"

if [[ "${1:-}" == "restart" ]]; then
  echo "🐺 Restarting $SERVICE..."
  systemctl --user restart "$SERVICE"
  sleep 2
  systemctl --user status "$SERVICE" --no-pager
  exit 0
fi

if [[ "${1:-}" == "tail" ]]; then
  echo "🐺 Werewolf Live Logs: wolf-b"
  echo "Ctrl+C to exit"
  echo
  exec journalctl --user -u "$SERVICE" -f
fi

if [[ "${1:-}" == "logs" ]]; then
  shift || true
  case "${1:-recent}" in
    --errors|errors)
      echo "🐺 Werewolf Logs (errors)"
      exec journalctl --user -u "$SERVICE" --no-pager | grep -Ei "error|failed|refused|timeout|panic|warn" || true
      ;;
    --quic|quic)
      echo "🐺 Werewolf Logs (quic)"
      exec journalctl --user -u "$SERVICE" --no-pager | grep -Ei "quic" || true
      ;;
    --tcp|tcp)
      echo "🐺 Werewolf Logs (tcp/fang)"
      exec journalctl --user -u "$SERVICE" --no-pager | grep -Ei "tcp|fang" || true
      ;;
    *)
      echo "🐺 Werewolf Logs (recent)"
      exec journalctl --user -u "$SERVICE" -n 80 --no-pager
      ;;
  esac
fi

if [[ "${1:-}" == "transport-check" ]]; then
  echo "🐺 Werewolf Transport Check (wolf-b)"
  echo "===================================="
  echo

  echo "🌐 Target HTTP"
  if curl --max-time 3 -fsS "$TARGET_URL" >/dev/null; then
    echo "  target:       reachable ✅"
  else
    echo "  target:       failed ❌"
  fi

  echo
  echo "🦷 QUIC Fang"
  if curl --max-time 5 -fsS "$QUIC_URL" >/dev/null; then
    echo "  quic:         healthy ✅"
    quic_ok=1
  else
    echo "  quic:         failed ❌"
    quic_ok=0
  fi

  echo
  echo "🦷 TCP Fallback"
  if curl --max-time 5 -fsS "$TCP_URL" >/dev/null; then
    echo "  tcp:          healthy ✅"
    tcp_ok=1
  else
    echo "  tcp:          failed ❌"
    tcp_ok=0
  fi

  echo
  echo "🔐 TCP Encrypted v2"
  if curl --max-time 5 -fsS "$TCP_ENC_URL" >/dev/null; then
    echo "  tcp-enc-v2:   healthy ✅"
    tcp_enc_ok=1
  else
    echo "  tcp-enc-v2:   failed ❌"
    tcp_enc_ok=0
  fi

  echo
  echo "📊 Recommendation"
  if [[ "$quic_ok" == "1" ]]; then
    echo "  transport:    QUIC primary 🟢"
    echo "  url:          $QUIC_URL"
  elif [[ "$tcp_ok" == "1" ]]; then
    echo "  transport:    TCP fallback 🟡"
    echo "  url:          $TCP_URL"
  else
    echo "  transport:    none available 🔴"
  fi

  exit 0
fi

if [[ "${1:-}" == "fallback-plan" ]]; then
  echo "🐺 Werewolf Fallback Plan (wolf-b)"
  echo "=================================="
  echo

  echo "Primary transport:"
  echo "  type:        QUIC"
  echo "  url:         $QUIC_URL"

  if curl --max-time 5 -fsS "$QUIC_URL" >/dev/null; then
    echo "  tunnel:      healthy ✅"
    quic_ok=1
  else
    echo "  tunnel:      failed ❌"
    quic_ok=0
  fi

  echo
  echo "Fallback transport:"
  echo "  type:        TCP plain"
  echo "  url:         $TCP_URL"

  if curl --max-time 5 -fsS "$TCP_URL" >/dev/null; then
    echo "  tunnel:      healthy ✅"
    tcp_ok=1
  else
    echo "  tunnel:      failed ❌"
    tcp_ok=0
  fi

  echo
  echo "Encrypted fallback transport:"
  echo "  type:        TCP encrypted v2"
  echo "  url:         $TCP_ENC_URL"

  if curl --max-time 5 -fsS "$TCP_ENC_URL" >/dev/null; then
    echo "  tunnel:      healthy ✅"
    tcp_enc_ok=1
  else
    echo "  tunnel:      failed ❌"
    tcp_enc_ok=0
  fi

  echo
  echo "Decision:"
  if [[ "$quic_ok" == "1" ]]; then
    echo "  action:      stay on QUIC 🟢"
    echo "  url:         $QUIC_URL"
  elif [[ "${tcp_enc_ok:-0}" == "1" ]]; then
    echo "  action:      fallback to TCP encrypted v2 🟡"
    echo "  url:         $TCP_ENC_URL"
  elif [[ "$tcp_ok" == "1" ]]; then
    echo "  action:      fallback to TCP plain 🟠"
    echo "  url:         $TCP_URL"
  else
    echo "  action:      no viable transport 🔴"
  fi

  exit 0
fi

if [[ "${1:-}" == "auto" ]]; then
  json_mode=0
  policy="${WEREWOLF_TRANSPORT_POLICY:-secure}"
  shift || true

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --json)
        json_mode=1
        ;;
      --policy)
        shift || true
        policy="${1:-secure}"
        ;;
      --policy=*)
        policy="${1#--policy=}"
        ;;
      *)
        ;;
    esac
    shift || true
  done

  case "$policy" in
    performance)
      order=("quic" "tcp-plain" "tcp-encrypted-v2")
      ;;
    secure)
      order=("quic" "tcp-encrypted-v2" "tcp-plain")
      ;;
    stealth)
      order=("tcp-encrypted-v2" "quic" "tcp-plain")
      ;;
    *)
      echo "Unknown policy: $policy" >&2
      exit 2
      ;;
  esac

  if [[ "$json_mode" == "0" ]]; then
    echo "🐺 Werewolf Auto Transport"
    echo "========================="
    echo
    echo "policy:    $policy"
  fi

  for transport in "${order[@]}"; do
    case "$transport" in
      quic)
        label="QUIC"
        url="$QUIC_URL"
        human="QUIC 🟢"
        ;;
      tcp-encrypted-v2)
        label="TCP encrypted v2"
        url="$TCP_ENC_URL"
        human="TCP encrypted v2 🟡"
        ;;
      tcp-plain)
        label="TCP plain fallback"
        url="$TCP_URL"
        human="TCP plain fallback 🟠"
        ;;
    esac

    if curl --max-time 5 -fsS "$url" >/dev/null; then
      if [[ "$json_mode" == "1" ]]; then
        printf '{"transport":"%s","label":"%s","url":"%s","healthy":true,"policy":"%s"}\n' "$transport" "$label" "$url" "$policy"
      else
        echo "transport: $human"
        echo "url:       $url"
      fi
      exit 0
    fi

    if [[ "$json_mode" == "0" ]]; then
      echo "⚠ $label failed"
    fi
  done

  if [[ "$json_mode" == "1" ]]; then
    printf '{"transport":"unavailable","label":"unavailable","url":null,"healthy":false,"policy":"%s"}\n' "$policy"
  else
    echo "transport: unavailable 🔴"
  fi

  exit 2
fi

if [[ "${1:-}" == "breathe" ]]; then
  echo "🐺 Werewolf Breath"
  echo "================="
  echo

  echo "🩺 Health"
  "$0" health-summary

  echo
  echo "🤖 Auto-heal JSON"
  "$0" auto-heal-json | jq .

  echo
  echo "🧠 Policy views"
  echo "secure:"
  "$0" auto --policy secure --json | jq .
  echo "performance:"
  "$0" auto --policy performance --json | jq .
  echo "stealth:"
  "$0" auto --policy stealth --json | jq .

  echo
  echo "🦷 Active Fangs"
  "$0" fang list

  echo
  echo "🐾 Pack"
  "$0" pack list

  echo
  echo "✅ Wolf is breathing"

  exit 0
fi

if [[ "${1:-}" == "watchdog-status" ]]; then
  echo "🐺 Werewolf B Watchdog Status"
  echo "============================"
  echo

  systemctl --user status werewolf-b-watchdog.timer --no-pager || true
  echo
  systemctl --user status werewolf-b-watchdog.service --no-pager || true
  echo
  journalctl --user -u werewolf-b-watchdog.service -n 30 --no-pager || true

  exit 0
fi

if [[ "${1:-}" == "watch" ]]; then
  interval="${2:-10}"

  echo "🐺 Werewolf Transport Watch"
  echo "=========================="
  echo "interval: ${interval}s"
  echo "Ctrl+C to stop"
  echo

  while true; do
    date '+%Y-%m-%d %H:%M:%S'
    "$0" heal --quiet >/dev/null 2>&1 || true
    "$0" health-summary
    echo
    sleep "$interval"
  done
fi

if [[ "${1:-}" == "auto-heal-json" ]]; then
  "$0" heal --quiet >/dev/null 2>&1 || true
  "$0" auto --json
  exit $?
fi

if [[ "${1:-}" == "heal" ]]; then
  quiet=0
  if [[ "${2:-}" == "--quiet" ]]; then
    quiet=1
  fi

  if [[ "$quiet" == "0" ]]; then
    echo "🐺 Werewolf Transport Heal"
    echo "========================="
    echo
  fi

  ensure_profile() {
    local name="$1"
    local port="$2"

    if wolf-b fang list | grep -q "local:  127.0.0.1:${port}"; then
      [[ "$quiet" == "0" ]] && echo "✅ $name already active on $port"
    else
      [[ "$quiet" == "0" ]] && echo "🔁 opening $name..."
      wolf-b fang open-profile "$name" >/dev/null || {
        [[ "$quiet" == "0" ]] && echo "❌ failed to open $name"
        return 1
      }
      [[ "$quiet" == "0" ]] && echo "✅ $name opened"
    fi
  }

  ensure_profile home-web-tcp 9021
  ensure_profile home-web-tcp-encrypted-v2 9022
  ensure_profile large-transfer-tcp-v2 9032
  ensure_profile home-web-quic 9020

  if [[ "$quiet" == "0" ]]; then
    echo
    "$0" health-summary
    exit 0
  fi

  "$0" auto-heal-json >/dev/null 2>&1 || true
  "$0" transport-health-json | jq -e '
    .transports.quic.healthy == true
    or .transports["tcp-encrypted-v2"].healthy == true
    or .transports["tcp-plain"].healthy == true
  ' >/dev/null
  exit $?
fi

if [[ "${1:-}" == "health-summary" ]]; then
  echo "🐺 Werewolf Transport Health"
  echo "==========================="
  echo

  data="$("$0" transport-health-json)"

  quic="$(echo "$data" | jq -r '.transports.quic.healthy')"
  tcp_plain="$(echo "$data" | jq -r '.transports["tcp-plain"].healthy')"
  tcp_enc="$(echo "$data" | jq -r '.transports["tcp-encrypted-v2"].healthy')"

  [[ "$quic" == "true" ]] && echo "⚡ QUIC:             healthy ✅" || echo "⚡ QUIC:             failed ❌"
  [[ "$tcp_enc" == "true" ]] && echo "🔐 TCP encrypted v2: healthy ✅" || echo "🔐 TCP encrypted v2: failed ❌"
  [[ "$tcp_plain" == "true" ]] && echo "🦷 TCP plain:        healthy ✅" || echo "🦷 TCP plain:        failed ❌"

  echo
  "$0" auto --json | jq .

  exit 0
fi

if [[ "${1:-}" == "transport-health-json" ]]; then
  quic_ok=false
  tcp_ok=false
  tcp_enc_ok=false

  curl --max-time 5 -fsS "$QUIC_URL" >/dev/null && quic_ok=true || true
  curl --max-time 5 -fsS "$TCP_URL" >/dev/null && tcp_ok=true || true
  curl --max-time 5 -fsS "$TCP_ENC_URL" >/dev/null && tcp_enc_ok=true || true

  jq -n \
    --arg quic_url "$QUIC_URL" \
    --arg tcp_url "$TCP_URL" \
    --arg tcp_enc_url "$TCP_ENC_URL" \
    --argjson quic_ok "$quic_ok" \
    --argjson tcp_ok "$tcp_ok" \
    --argjson tcp_enc_ok "$tcp_enc_ok" \
    '{
      transports: {
        quic: {
          url: $quic_url,
          healthy: $quic_ok
        },
        "tcp-plain": {
          url: $tcp_url,
          healthy: $tcp_ok
        },
        "tcp-encrypted-v2": {
          url: $tcp_enc_url,
          healthy: $tcp_enc_ok
        }
      }
    }'

  exit 0
fi

if [[ "${1:-}" == "benchmark" ]]; then
  json_mode=0
  if [[ "${2:-}" == "--json" ]]; then
    json_mode=1
  fi

  direct=$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$TARGET_URL" || echo "fail")
  quic=$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$QUIC_URL" || echo "fail")
  tcp=$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$TCP_URL" || echo "fail")
  tcp_enc=$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$TCP_ENC_URL" || echo "fail")

  if [[ "$json_mode" == "1" ]]; then
    jq -n \
      --arg target "$TARGET_URL" \
      --arg quic_url "$QUIC_URL" \
      --arg tcp_url "$TCP_URL" \
      --arg tcp_enc_url "$TCP_ENC_URL" \
      --arg direct "$direct" \
      --arg quic "$quic" \
      --arg tcp "$tcp" \
      --arg tcp_enc "$tcp_enc" \
      '{
        target: {
          url: $target,
          latency_seconds: (if $direct == "fail" then null else ($direct | tonumber) end),
          healthy: ($direct != "fail")
        },
        transports: {
          quic: {
            url: $quic_url,
            latency_seconds: (if $quic == "fail" then null else ($quic | tonumber) end),
            healthy: ($quic != "fail")
          },
          "tcp-plain": {
            url: $tcp_url,
            latency_seconds: (if $tcp == "fail" then null else ($tcp | tonumber) end),
            healthy: ($tcp != "fail")
          },
          "tcp-encrypted-v2": {
            url: $tcp_enc_url,
            latency_seconds: (if $tcp_enc == "fail" then null else ($tcp_enc | tonumber) end),
            healthy: ($tcp_enc != "fail")
          }
        }
      }'
    exit 0
  fi

  echo "🐺 Werewolf Benchmark (wolf-b)"
  echo "================================"
  echo
  echo "target: $TARGET_URL"
  echo "quic:   $QUIC_URL"
  echo "tcp:    $TCP_URL"
  echo "tcp-v2: $TCP_ENC_URL"
  echo

  echo "🌐 Direct target latency"
  echo "  direct: $direct s"
  echo
  echo "⚡ QUIC Fang latency"
  echo "  quic:   $quic s"
  echo
  echo "🦷 TCP fallback latency"
  echo "  tcp:    $tcp s"
  echo
  echo "🔐 TCP encrypted v2 latency"
  echo "  tcp-v2: $tcp_enc s"
  exit 0
fi

exec werewolfctl --socket "$SOCKET" "$@"
