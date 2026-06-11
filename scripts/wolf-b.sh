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
  policy="secure"

  shift || true

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --json)
        json_mode=1
        ;;
      --policy)
        policy="${2:-secure}"
        shift
        ;;
    esac
    shift || true
  done

  if [[ "$json_mode" == "0" ]]; then
    echo "🐺 Werewolf Auto Transport"
    echo "========================="
    echo
    echo "policy:    $policy"
  fi

  choose_transport() {
    local transport="$1"
    local label="$2"
    local url="$3"

    if [[ "$json_mode" == "1" ]]; then
      printf '{"transport":"%s","label":"%s","url":"%s","healthy":true,"policy":"%s"}\n' \
        "$transport" "$label" "$url" "$policy"
    else
      echo
      echo "transport: $label 🟢"
      echo "url:       $url"
    fi
    exit 0
  }

  if [[ "$policy" == "performance" ]]; then
    data="$("$0" benchmark --json)"

    best_transport=""
    best_latency=999999

    for t in quic tcp-encrypted-v2 tcp-plain; do
      healthy="$(echo "$data" | jq -r ".transports[\"$t\"].healthy")"
      latency="$(echo "$data" | jq -r ".transports[\"$t\"].latency_seconds")"

      [[ "$healthy" != "true" ]] && continue
      [[ "$latency" == "null" ]] && continue

      better=$(awk "BEGIN {print ($latency < $best_latency)}")

      if [[ "$better" == "1" ]]; then
        best_latency="$latency"
        best_transport="$t"
      fi
    done

    case "$best_transport" in
      quic)
        choose_transport "quic" "QUIC" "$QUIC_URL"
        ;;
      tcp-encrypted-v2)
        choose_transport "tcp-encrypted-v2" "TCP encrypted v2" "$TCP_ENC_URL"
        ;;
      tcp-plain)
        choose_transport "tcp-plain" "TCP plain fallback" "$TCP_URL"
        ;;
    esac
  fi

  if [[ "$policy" == "stealth" ]]; then
    curl --max-time 5 -fsS "$TCP_ENC_URL" >/dev/null && \
      choose_transport "tcp-encrypted-v2" "TCP encrypted v2" "$TCP_ENC_URL"

    curl --max-time 5 -fsS "$TCP_URL" >/dev/null && \
      choose_transport "tcp-plain" "TCP plain fallback" "$TCP_URL"

    curl --max-time 5 -fsS "$QUIC_URL" >/dev/null && \
      choose_transport "quic" "QUIC" "$QUIC_URL"
  fi

  curl --max-time 5 -fsS "$QUIC_URL" >/dev/null && \
    choose_transport "quic" "QUIC" "$QUIC_URL"

  curl --max-time 5 -fsS "$TCP_ENC_URL" >/dev/null && \
    choose_transport "tcp-encrypted-v2" "TCP encrypted v2" "$TCP_ENC_URL"

  curl --max-time 5 -fsS "$TCP_URL" >/dev/null && \
    choose_transport "tcp-plain" "TCP plain fallback" "$TCP_URL"

  if [[ "$json_mode" == "1" ]]; then
    printf '{"transport":"unavailable","label":"unavailable","url":null,"healthy":false,"policy":"%s"}\n' "$policy"
  else
    echo
    echo "transport: unavailable 🔴"
  fi

  exit 2
fi

if [[ "${1:-}" == "benchmark" ]]; then
  json_mode=0
  if [[ "${2:-}" == "--json" ]]; then
    json_mode=1
  fi

  measure_latency() {
    local url="$1"
    local out
    if out=$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$url"); then
      echo "$out"
    else
      echo "fail"
    fi
  }

  direct="$(measure_latency "$TARGET_URL")"
  quic="$(measure_latency "$QUIC_URL")"
  tcp="$(measure_latency "$TCP_URL")"
  tcp_enc="$(measure_latency "$TCP_ENC_URL")"

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
