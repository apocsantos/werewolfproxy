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
  echo "🐺 Werewolf Auto Transport"
  echo "========================="
  echo

  if curl --max-time 5 -fsS "$QUIC_URL" >/dev/null; then
    echo "transport: QUIC 🟢"
    echo "url:       $QUIC_URL"
    exit 0
  fi

  echo "⚠️ QUIC failed, trying TCP encrypted v2..."

  if curl --max-time 5 -fsS "$TCP_ENC_URL" >/dev/null; then
    echo "transport: TCP encrypted v2 🟡"
    echo "url:       $TCP_ENC_URL"
    exit 0
  fi

  echo "⚠️ TCP encrypted v2 failed, trying TCP plain fallback..."

  if curl --max-time 5 -fsS "$TCP_URL" >/dev/null; then
    echo "transport: TCP plain fallback 🟠"
    echo "url:       $TCP_URL"
    exit 0
  fi

  echo "transport: unavailable 🔴"
  echo "quic:      failed"
  echo "tcp-enc:   failed"
  echo "tcp:       failed"
  exit 2
fi

if [[ "${1:-}" == "benchmark" ]]; then
  echo "🐺 Werewolf Benchmark (wolf-b)"
  echo "================================"
  echo
  echo "target: $TARGET_URL"
  echo "quic:   $QUIC_URL"
  echo "tcp:    $TCP_URL"
  echo "tcp-v2: $TCP_ENC_URL"
  echo

  direct=$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$TARGET_URL" || echo "fail")
  quic=$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$QUIC_URL" || echo "fail")
  tcp=$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$TCP_URL" || echo "fail")
  tcp_enc=$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$TCP_ENC_URL" || echo "fail")

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
