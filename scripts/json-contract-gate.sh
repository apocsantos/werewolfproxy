#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf JSON Contract Gate"
echo "============================="
echo

failures=0

pass() {
  echo "✅ $1"
}

fail() {
  echo "❌ $1"
  failures=$((failures + 1))
}

validate() {
  local name="$1"
  local cmd="$2"
  local jq_check="$3"

  echo "🧪 $name"

  if eval "$cmd" >/tmp/wolf-json-test.json 2>/tmp/wolf-json-test.err; then
    cat /tmp/wolf-json-test.json | jq .

    if jq -e "$jq_check" /tmp/wolf-json-test.json >/dev/null; then
      pass "$name contract valid"
    else
      fail "$name contract invalid"
    fi
  else
    cat /tmp/wolf-json-test.err || true
    fail "$name execution failed"
  fi

  echo
}

validate \
  "doctor" \
  "wolf-b doctor --json" \
  '.healthy == true and (.checks | length > 0)'

validate \
  "ready" \
  "wolf-b ready --json" \
  '.ready == true and .doctor.healthy == true'

validate \
  "selftest" \
  "wolf-b selftest --json" \
  '.ok == true'

validate \
  "status" \
  "wolf-b status-json" \
  '.ready.ready == true and (.policy_test | length > 0)'

validate \
  "cache" \
  "wolf-b cache-status --json" \
  '.scores != null and .snapshots != null and .reports != null'

validate \
  "benchmark" \
  "wolf-b benchmark --json" \
  '.transports.quic != null'

echo
if [[ "$failures" == "0" ]]; then
  echo "🎉 JSON CONTRACT GATE PASSED"
  exit 0
else
  echo "⚠ JSON CONTRACT GATE FAILED ($failures)"
  exit 1
fi
