#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Service HTTP Gate"
echo "============================"
echo

./scripts/test-targets.sh
./scripts/start-multiwolf.sh
./scripts/peer-db.sh seed-local

port="${1:-18082}"

./scripts/service-db.sh add gate-http wolf-a 127.0.0.1:8080 http >/dev/null
./scripts/service-db.sh connect gate-http "$port" >/tmp/wolf-service-http-connect.txt
sleep 2

curl --max-time 5 -fsS "http://127.0.0.1:$port/" >/tmp/wolf-service-http.html

if grep -qi "Directory listing" /tmp/wolf-service-http.html; then
  echo "✅ HTTP service over Werewolf works"
else
  echo "❌ HTTP service response unexpected"
  exit 1
fi

echo
echo "🎉 SERVICE HTTP GATE PASSED"
