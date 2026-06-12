#!/usr/bin/env bash
set -euo pipefail

SOCKET="/tmp/wolf-b.sock"
SERVICE="werewolf-b"

TARGET_URL="${WEREWOLF_TARGET_URL:-http://127.0.0.1:8080}"
QUIC_URL="${WEREWOLF_QUIC_URL:-http://127.0.0.1:9020}"
TCP_URL="${WEREWOLF_TCP_URL:-http://127.0.0.1:9021}"
TCP_ENC_URL="${WEREWOLF_TCP_ENC_URL:-http://127.0.0.1:9022}"

is_healthy() {
  curl --max-time 5 -fsS "$1" >/dev/null
}

measure_latency() {
  local url="$1"
  local out
  if out="$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$url")"; then
    echo "$out"
  else
    echo "fail"
  fi
}

choose_json() {
  printf '{"transport":"%s","label":"%s","url":%s,"healthy":%s,"policy":"%s"}\n' \
    "$1" "$2" "$3" "$4" "${5:-secure}"
}

if [[ "${1:-}" == "restart" ]]; then
  echo "🐺 Restarting $SERVICE..."
  systemctl --user restart "$SERVICE"
  sleep 2
  systemctl --user status "$SERVICE" --no-pager
  exit 0
fi

if [[ "${1:-}" == "tail" ]]; then
  exec journalctl --user -u "$SERVICE" -f
fi

if [[ "${1:-}" == "logs" ]]; then
  shift || true
  case "${1:-recent}" in
    errors|--errors) exec journalctl --user -u "$SERVICE" --no-pager | grep -Ei "error|failed|refused|timeout|panic|warn" || true ;;
    quic|--quic) exec journalctl --user -u "$SERVICE" --no-pager | grep -Ei "quic" || true ;;
    tcp|--tcp) exec journalctl --user -u "$SERVICE" --no-pager | grep -Ei "tcp|fang" || true ;;
    *) exec journalctl --user -u "$SERVICE" -n 80 --no-pager ;;
  esac
fi

if [[ "${1:-}" == "transport-health-json" ]]; then
  quic_ok=false; tcp_ok=false; tcp_enc_ok=false
  is_healthy "$QUIC_URL" && quic_ok=true || true
  is_healthy "$TCP_URL" && tcp_ok=true || true
  is_healthy "$TCP_ENC_URL" && tcp_enc_ok=true || true

  jq -n \
    --arg quic_url "$QUIC_URL" \
    --arg tcp_url "$TCP_URL" \
    --arg tcp_enc_url "$TCP_ENC_URL" \
    --argjson quic_ok "$quic_ok" \
    --argjson tcp_ok "$tcp_ok" \
    --argjson tcp_enc_ok "$tcp_enc_ok" \
    '{transports:{quic:{url:$quic_url,healthy:$quic_ok},"tcp-plain":{url:$tcp_url,healthy:$tcp_ok},"tcp-encrypted-v2":{url:$tcp_enc_url,healthy:$tcp_enc_ok}}}'
  exit 0
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

if [[ "${1:-}" == "heal" ]]; then
  quiet=0
  [[ "${2:-}" == "--quiet" ]] && quiet=1

  [[ "$quiet" == "0" ]] && echo "🐺 Werewolf Transport Heal" && echo "=========================" && echo

  ensure_profile() {
    local name="$1"
    local port="$2"

    if "$0" fang list | grep -q "local:  127.0.0.1:${port}"; then
      [[ "$quiet" == "0" ]] && echo "✅ $name already active on $port"
    else
      [[ "$quiet" == "0" ]] && echo "🔁 opening $name..."
      "$0" fang open-profile "$name" >/dev/null || true
    fi
  }

  ensure_profile home-web-tcp 9021
  ensure_profile home-web-tcp-encrypted-v2 9022
  ensure_profile large-transfer-tcp-v2 9032
  ensure_profile home-web-quic 9020

  [[ "$quiet" == "0" ]] && echo && "$0" health-summary
  exit 0
fi

if [[ "${1:-}" == "auto-heal-json" ]]; then
  "$0" heal --quiet >/dev/null 2>&1 || true
  "$0" auto --json
  exit $?
fi

if [[ "${1:-}" == "transport-check" ]]; then
  echo "🐺 Werewolf Transport Check (wolf-b)"
  echo "===================================="
  echo
  "$0" health-summary
  exit 0
fi

if [[ "${1:-}" == "fallback-plan" ]]; then
  echo "🐺 Werewolf Fallback Plan (wolf-b)"
  echo "=================================="
  echo
  "$0" health-summary
  exit 0
fi

if [[ "${1:-}" == "score-history" ]]; then
  score_file="${WEREWOLF_SCORE_FILE:-$HOME/.cache/werewolf/wolf-b-scores.jsonl}"

  if [[ ! -f "$score_file" ]]; then
    echo "❌ no score history found"
    exit 1
  fi

  echo "🐺 Werewolf Transport Trends"
  echo "============================"
  echo

  analyze_transport() {
    local transport="$1"

    avg_latency="$(jq -s --arg t "$transport" '
      map(.transports[$t].latency_seconds // empty)
      | if length == 0 then null else (add / length) end
    ' "$score_file")"

    avg_score="$(jq -s --arg t "$transport" '
      map(.transports[$t].score // empty)
      | if length == 0 then null else (add / length) end
    ' "$score_file")"

    healthy_pct="$(jq -s --arg t "$transport" '
      map(.transports[$t].healthy)
      | if length == 0 then 0
        else ((map(select(. == true)) | length) / length * 100)
      end
    ' "$score_file")"

    echo "$transport"
    printf "  avg latency:   %.2f ms\n" "$(awk "BEGIN {print ($avg_latency // 0) * 1000}")"
    printf "  avg score:     %.2f\n" "$(echo "$avg_score" | awk '{print $1+0}')"
    printf "  healthy:       %.1f%%\n" "$(echo "$healthy_pct" | awk '{print $1+0}')"
    echo
  }

  analyze_transport quic
  analyze_transport tcp-encrypted-v2
  analyze_transport tcp-plain

  echo "recommended:"
  "$0" auto --policy resilience --json | jq -r '
    "  " + .label
  '

  exit 0
fi

if [[ "${1:-}" == "score-save" ]]; then
  mkdir -p ~/.cache/werewolf
  score_file="${WEREWOLF_SCORE_FILE:-$HOME/.cache/werewolf/wolf-b-scores.jsonl}"

  "$0" score-json | jq -c --arg ts "$(date -Iseconds)" '. + {timestamp: $ts}' >> "$score_file"

  echo "✅ score saved: $score_file"
  tail -n 1 "$score_file" | jq .

  exit 0
fi

if [[ "${1:-}" == "score" ]]; then
  echo "🐺 Werewolf Transport Scores"
  echo "==========================="
  echo

  data="$("$0" score-json)"

  for t in quic tcp-encrypted-v2 tcp-plain; do
    healthy="$(echo "$data" | jq -r ".transports[\"$t\"].healthy")"
    latency="$(echo "$data" | jq -r ".transports[\"$t\"].latency_seconds")"
    score="$(echo "$data" | jq -r ".transports[\"$t\"].score")"

    printf "%-18s healthy=%-5s latency=%-10s score=%s\n" "$t" "$healthy" "$latency" "$score"
  done

  echo
  "$0" auto --policy resilience --json | jq .

  exit 0
fi

if [[ "${1:-}" == "score-json" ]]; then
  health="$("$0" transport-health-json)"
  bench="$("$0" benchmark --json)"

  jq -n \
    --argjson health "$health" \
    --argjson bench "$bench" \
    '{
      transports: {
        quic: {
          healthy: $health.transports.quic.healthy,
          latency_seconds: $bench.transports.quic.latency_seconds,
          score: (
            if $health.transports.quic.healthy == true then
              100 - (($bench.transports.quic.latency_seconds // 1) * 1000)
            else 0 end
          )
        },
        "tcp-encrypted-v2": {
          healthy: $health.transports["tcp-encrypted-v2"].healthy,
          latency_seconds: $bench.transports["tcp-encrypted-v2"].latency_seconds,
          score: (
            if $health.transports["tcp-encrypted-v2"].healthy == true then
              90 - (($bench.transports["tcp-encrypted-v2"].latency_seconds // 1) * 1000)
            else 0 end
          )
        },
        "tcp-plain": {
          healthy: $health.transports["tcp-plain"].healthy,
          latency_seconds: $bench.transports["tcp-plain"].latency_seconds,
          score: (
            if $health.transports["tcp-plain"].healthy == true then
              70 - (($bench.transports["tcp-plain"].latency_seconds // 1) * 1000)
            else 0 end
          )
        }
      }
    }'

  exit 0
fi

if [[ "${1:-}" == "auto" ]]; then
  json_mode=0
  policy="${WEREWOLF_TRANSPORT_POLICY:-secure}"

  shift || true
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --json) json_mode=1 ;;
      --policy) shift || true; policy="${1:-secure}" ;;
      --policy=*) policy="${1#--policy=}" ;;
    esac
    shift || true
  done

  emit_choice() {
    local transport="$1"
    local label="$2"
    local url="$3"
    if [[ "$json_mode" == "1" ]]; then
      printf '{"transport":"%s","label":"%s","url":"%s","healthy":true,"policy":"%s"}\n' "$transport" "$label" "$url" "$policy"
    else
      echo "transport: $label"
      echo "url:       $url"
    fi
    exit 0
  }

  [[ "$json_mode" == "0" ]] && echo "🐺 Werewolf Auto Transport" && echo "=========================" && echo && echo "policy:    $policy"

  if [[ "$policy" == "learned" ]]; then
    score_file="${WEREWOLF_SCORE_FILE:-$HOME/.cache/werewolf/wolf-b-scores.jsonl}"

    if [[ -f "$score_file" ]]; then
      best_transport="$(jq -s -r '
        {
          quic: (map(.transports.quic.score // empty) | if length == 0 then -999999 else add / length end),
          "tcp-encrypted-v2": (map(.transports["tcp-encrypted-v2"].score // empty) | if length == 0 then -999999 else add / length end),
          "tcp-plain": (map(.transports["tcp-plain"].score // empty) | if length == 0 then -999999 else add / length end)
        }
        | to_entries
        | sort_by(.value)
        | reverse
        | .[0].key
      ' "$score_file")"

      case "$best_transport" in
        quic)
          is_healthy "$QUIC_URL" && emit_choice "quic" "QUIC 🟢" "$QUIC_URL"
          ;;
        tcp-encrypted-v2)
          is_healthy "$TCP_ENC_URL" && emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL"
          ;;
        tcp-plain)
          is_healthy "$TCP_URL" && emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL"
          ;;
      esac
    fi

    policy="resilience"
  fi

  if [[ "$policy" == "resilience" ]]; then
    data="$("$0" score-json)"

    best_transport=""
    best_score="-999999"

    for t in quic tcp-encrypted-v2 tcp-plain; do
      healthy="$(echo "$data" | jq -r ".transports[\"$t\"].healthy")"
      score="$(echo "$data" | jq -r ".transports[\"$t\"].score")"

      [[ "$healthy" != "true" ]] && continue

      if awk "BEGIN {exit !($score > $best_score)}"; then
        best_score="$score"
        best_transport="$t"
      fi
    done

    case "$best_transport" in
      quic) emit_choice "quic" "QUIC 🟢" "$QUIC_URL" ;;
      tcp-encrypted-v2) emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL" ;;
      tcp-plain) emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL" ;;
    esac
  fi

  if [[ "$policy" == "performance" ]]; then
    data="$("$0" benchmark --json)"
    best_transport=""
    best_latency="999999"

    for t in quic tcp-encrypted-v2 tcp-plain; do
      healthy="$(echo "$data" | jq -r ".transports[\"$t\"].healthy")"
      latency="$(echo "$data" | jq -r ".transports[\"$t\"].latency_seconds")"
      [[ "$healthy" != "true" || "$latency" == "null" ]] && continue

      if awk "BEGIN {exit !($latency < $best_latency)}"; then
        best_latency="$latency"
        best_transport="$t"
      fi
    done

    case "$best_transport" in
      quic) emit_choice "quic" "QUIC 🟢" "$QUIC_URL" ;;
      tcp-encrypted-v2) emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL" ;;
      tcp-plain) emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL" ;;
    esac
  fi

  if [[ "$policy" == "stealth" ]]; then
    is_healthy "$TCP_ENC_URL" && emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL"
    is_healthy "$TCP_URL" && emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL"
    is_healthy "$QUIC_URL" && emit_choice "quic" "QUIC 🟢" "$QUIC_URL"
  fi

  is_healthy "$QUIC_URL" && emit_choice "quic" "QUIC 🟢" "$QUIC_URL"
  is_healthy "$TCP_ENC_URL" && emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL"
  is_healthy "$TCP_URL" && emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL"

  if [[ "$json_mode" == "1" ]]; then
    printf '{"transport":"unavailable","label":"unavailable","url":null,"healthy":false,"policy":"%s"}\n' "$policy"
  else
    echo "transport: unavailable 🔴"
  fi
  exit 2
fi

if [[ "${1:-}" == "benchmark" ]]; then
  json_mode=0
  [[ "${2:-}" == "--json" ]] && json_mode=1

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
        target:{url:$target,latency_seconds:(if $direct=="fail" then null else ($direct|tonumber) end),healthy:($direct!="fail")},
        transports:{
          quic:{url:$quic_url,latency_seconds:(if $quic=="fail" then null else ($quic|tonumber) end),healthy:($quic!="fail")},
          "tcp-plain":{url:$tcp_url,latency_seconds:(if $tcp=="fail" then null else ($tcp|tonumber) end),healthy:($tcp!="fail")},
          "tcp-encrypted-v2":{url:$tcp_enc_url,latency_seconds:(if $tcp_enc=="fail" then null else ($tcp_enc|tonumber) end),healthy:($tcp_enc!="fail")}
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

exec werewolfctl --socket "$SOCKET" "$@"
