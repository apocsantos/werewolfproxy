#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Service TCP Gate"
echo "==========================="
echo

target_port="${1:-19090}"
local_port="${2:-19091}"

pkill -f "werewolf_tcp_target_${target_port}" 2>/dev/null || true

cat > "/tmp/werewolf_tcp_target_${target_port}.py" <<PY
import socket

HOST = "127.0.0.1"
PORT = int("${target_port}")

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind((HOST, PORT))
    s.listen(50)
    while True:
        conn, addr = s.accept()
        with conn:
            conn.sendall(b"werewolf-tcp-ok\\n")
PY

python3 "/tmp/werewolf_tcp_target_${target_port}.py" >/tmp/werewolf-tcp-target.log 2>&1 &
target_pid="$!"

cleanup() {
  kill "$target_pid" 2>/dev/null || true
}
trap cleanup EXIT

sleep 1

./scripts/start-multiwolf.sh >/dev/null
./scripts/peer-db.sh seed-local >/dev/null

./scripts/service-db.sh add gate-tcp wolf-a "127.0.0.1:$target_port" tcp >/dev/null
./scripts/service-db.sh connect gate-tcp "$local_port" >/tmp/wolf-service-tcp-connect.txt

sleep 2

response="$(timeout 5 bash -lc "exec 3<>/dev/tcp/127.0.0.1/$local_port; printf 'ping\\n' >&3; head -n 1 <&3" || true)"

if [[ "$response" == *"werewolf-tcp-ok"* ]]; then
  echo "✅ Generic TCP service over Werewolf works"
else
  echo "❌ Generic TCP response unexpected"
  echo "response: $response"
  echo
  echo "connect log:"
  cat /tmp/wolf-service-tcp-connect.txt || true
  echo
  echo "target log:"
  cat /tmp/werewolf-tcp-target.log || true
  exit 1
fi

echo
echo "🎉 SERVICE TCP GATE PASSED"
