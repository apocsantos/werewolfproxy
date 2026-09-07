#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Service Gate"
echo "======================="
echo

failures=0
pass() { echo "✅ $1"; }
fail() { echo "❌ $1"; failures=$((failures + 1)); }

./scripts/start-multiwolf.sh >/dev/null
./scripts/peer-db.sh seed-local >/dev/null

./scripts/service-db.sh add home-ssh wolf-c 127.0.0.1:22 ssh >/dev/null
./scripts/service-db.sh add home-http wolf-c 127.0.0.1:80 http >/dev/null

if ./scripts/service-db.sh list | jq -e '."home-ssh" != null and ."home-http" != null' >/dev/null; then
  pass "services registered"
else
  fail "services missing"
fi

./scripts/service-db.sh route home-ssh >/tmp/wolf-service-route.json

if jq -e '.service == "home-ssh" and .peer == "wolf-c" and .kind == "ssh"' /tmp/wolf-service-route.json >/dev/null; then
  pass "service route shape valid"
else
  fail "service route shape invalid"
fi

if jq -e '.available == true' /tmp/wolf-service-route.json >/dev/null; then
  pass "service route available"
else
  fail "service route unavailable"
fi

if ./scripts/service-contract.sh >/tmp/wolf-service-contract.json; then
  if jq -e '.service_count >= 1 and (.services | length >= 1)' /tmp/wolf-service-contract.json >/dev/null; then
    pass "service contract valid"
  else
    fail "service contract invalid"
  fi
else
  fail "service contract failed"
fi

echo
cat /tmp/wolf-service-route.json | jq .

echo
if [[ "$failures" == "0" ]]; then
  echo "🎉 SERVICE GATE PASSED"
  exit 0
else
  echo "⚠ SERVICE GATE FAILED ($failures)"
  exit 1
fi
