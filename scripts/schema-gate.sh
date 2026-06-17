#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Schema Gate"
echo "======================"
echo

failures=0

pass() {
  echo "✅ $1"
}

fail() {
  echo "❌ $1"
  failures=$((failures + 1))
}

validate_shape() {
  local name="$1"
  local cmd="$2"
  local contract="$3"

  echo "🧪 $name"

  eval "$cmd" >/tmp/schema-current.json

  if jq -S 'paths | map(tostring) | join(".")' "$contract" >/tmp/schema-ref.txt \
    && jq -S 'paths | map(tostring) | join(".")' /tmp/schema-current.json >/tmp/schema-current.txt; then

    if diff -u /tmp/schema-ref.txt /tmp/schema-current.txt >/dev/null; then
      pass "$name schema stable"
    else
      echo
      diff -u /tmp/schema-ref.txt /tmp/schema-current.txt || true
      echo
      fail "$name schema drift detected"
    fi
  else
    fail "$name schema compare failed"
  fi

  echo
}

validate_shape \
  "doctor" \
  "wolf-b doctor --json" \
  "contracts/doctor.json"

validate_shape \
  "ready" \
  "wolf-b ready --json" \
  "contracts/ready.json"

validate_shape \
  "selftest" \
  "wolf-b selftest --json" \
  "contracts/selftest.json"

validate_shape \
  "status" \
  "wolf-b status-json" \
  "contracts/status.json"

validate_shape \
  "cache-status" \
  "wolf-b cache-status --json" \
  "contracts/cache-status.json"

validate_shape \
  "benchmark" \
  "wolf-b benchmark --json" \
  "contracts/benchmark.json"


validate_shape \
  "dashboard" \
  "./scripts/wolf-dashboard-json.sh" \
  "contracts/dashboard.json"

echo
if [[ "$failures" == "0" ]]; then
  echo "🎉 SCHEMA GATE PASSED"
  exit 0
else
  echo "⚠ SCHEMA GATE FAILED ($failures)"
  exit 1
fi
