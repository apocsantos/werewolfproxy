#!/usr/bin/env bash
set -e

SOCKET="/tmp/wolf-a.sock"
SERVICE="werewolf-a"
TARGET_URL="${WEREWOLF_TARGET_URL:-http://127.0.0.1:8080}"
TCP_LISTEN="${WEREWOLF_A_TCP_LISTEN:-127.0.0.1:8443}"
QUIC_LISTEN="${WEREWOLF_A_QUIC_LISTEN:-127.0.0.1:9560}"

if [[ "${1:-}" == "restart" ]]; then
  echo "🐺 Restarting $SERVICE..."
  systemctl --user restart "$SERVICE"
  sleep 2
  systemctl --user status "$SERVICE" --no-pager
  exit 0
fi

if [[ "${1:-}" == "tail" ]]; then
  echo "🐺 Werewolf Live Logs: wolf-a"
  echo "Ctrl+C to exit"
  echo
  exec journalctl --user -u "$SERVICE" -f
fi

if [[ "${1:-}" == "logs" ]]; then
  shift || true
  case "${1:-recent}" in
--errors|errors)
      echo "🐺 Werewolf Logs (errors)"
      journalctl --user -u "$SERVICE" --no-pager \
        | grep -Ei "error|failed|refused|timeout|panic|warn" || true
      exit 0
      ;;
    --quic|quic)
      echo "🐺 Werewolf Logs (quic)"
      journalctl --user -u "$SERVICE" --no-pager \
        | grep -Ei "quic" || true
      exit 0
      ;;
    --tcp|tcp)
      echo "🐺 Werewolf Logs (tcp/fang)"
      journalctl --user -u "$SERVICE" --no-pager \
        | grep -Ei "tcp|fang" || true
      exit 0
      ;;
    *)
      echo "🐺 Werewolf Logs (recent)"
      exec journalctl --user -u "$SERVICE" -n 80 --no-pager
      ;;
  esac
fi

if [[ "${1:-}" == "transport-check" ]]; then
  echo "🐺 Werewolf Transport Check (wolf-a)"
  echo "===================================="
  echo

  echo "🌐 Target HTTP"
  if curl --max-time 3 -fsS "$TARGET_URL" >/dev/null; then
    echo "  target:       reachable ✅"
  else
    echo "  target:       failed ❌"
  fi

  echo
  echo "🧰 Control socket"
  if [[ -S "$SOCKET" ]]; then
    echo "  socket:       present ✅"
  else
    echo "  socket:       missing ❌"
  fi

  echo
  echo "🦷 TCP listener"
  if ss -ltn | grep -q "$TCP_LISTEN"; then
    echo "  tcp:          $TCP_LISTEN ✅"
  else
    echo "  tcp:          $TCP_LISTEN ❌"
  fi

  echo
  echo "⚡ QUIC listener"
  if ss -lun | grep -q "$QUIC_LISTEN"; then
    echo "  quic:         $QUIC_LISTEN ✅"
  else
    echo "  quic:         $QUIC_LISTEN ❌"
  fi

  echo
  echo "📊 Recommendation"
  echo "  role:         receiver/listener"
  echo "  status:       ready if TCP + QUIC + socket are present"
  exit 0
fi

if [[ "${1:-}" == "benchmark" ]]; then
  echo "🐺 Werewolf Receiver Benchmark (wolf-a)"
  echo "======================================="
  echo
  echo "target: $TARGET_URL"
  echo

  direct=$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$TARGET_URL" || echo "fail")

  echo "🌐 Direct target latency"
  echo "  direct: $direct s"
  echo
  echo "📊 Receiver note"
  echo "  wolf-a listens for incoming TCP/QUIC Fangs."
  echo "  full tunnel benchmark should be run from wolf-b."
  exit 0
fi

exec werewolfctl --socket "$SOCKET" "$@"
