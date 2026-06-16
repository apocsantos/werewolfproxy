#!/usr/bin/env bash
set -euo pipefail

iterations="${1:-5}"
delay="${2:-5}"

echo "🐺 Werewolf Soak Gate"
echo "===================="
echo "iterations: $iterations"
echo "delay:      ${delay}s"
echo

failures=0

pass() {
  echo "✅ $1"
}

fail() {
  echo "❌ $1"
  failures=$((failures + 1))
}

./scripts/test-targets.sh
wolf-b heal --quiet || true

for i in $(seq 1 "$iterations"); do
  echo
  echo "🌕 Soak iteration $i/$iterations"
  echo "------------------------------"

  if wolf-b selftest --json | jq -e '.ok == true' >/dev/null; then
    pass "selftest ok"
  else
    fail "selftest failed"
  fi

  if wolf-b ready --json | jq -e '.ready == true' >/dev/null; then
    pass "ready ok"
  else
    fail "ready failed"
  fi

  wolf-b score-save >/dev/null || fail "score-save failed"
  wolf-b snapshot >/dev/null || fail "snapshot failed"

  if wolf-b snapshot-alert 20 >/dev/null; then
    pass "snapshot drift ok"
  else
    fail "snapshot drift alert"
  fi

  wolf-b policy-test

  sleep "$delay"
done

echo
if [[ "$failures" == "0" ]]; then
  echo "🎉 SOAK GATE PASSED"
  exit 0
else
  echo "⚠ SOAK GATE FAILED ($failures)"
  exit 1
fi
