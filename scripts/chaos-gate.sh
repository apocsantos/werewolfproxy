#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Chaos Gate"
echo "====================="
echo

failures=0

pass() {
  echo "✅ $1"
}

fail() {
  echo "❌ $1"
  failures=$((failures + 1))
}

echo "🔧 Pre-heal"
wolf-b heal --quiet || true
sleep 1

echo
echo "🩺 Baseline selftest"
if wolf-b selftest --json | jq -e '.ok == true' >/dev/null; then
  pass "baseline healthy"
else
  fail "baseline unhealthy"
fi

echo
echo "⚡ Simulating QUIC Fang outage"

quic_id="$(
  wolf-b fang list \
    | awk '
      /- fang_/ {id=$2}
      /transport: quic/ {print id; exit}
    '
)"

if [[ -z "${quic_id:-}" ]]; then
  fail "could not find active QUIC Fang"
else
  echo "closing QUIC Fang: $quic_id"
  wolf-b fang close "$quic_id" >/dev/null || true
  sleep 2
fi

echo
echo "🧪 Fallback check"

transport="$(wolf-b auto --policy secure --json | jq -r '.transport')"

if [[ "$transport" == "tcp-encrypted-v2" ]]; then
  pass "fallback switched to tcp-encrypted-v2"
elif [[ "$transport" == "tcp-plain" ]]; then
  pass "fallback switched to tcp-plain"
else
  fail "fallback transport unexpected: $transport"
fi

echo
echo "🔧 Healing"
wolf-b heal --quiet || true
sleep 3

echo
echo "🧪 Recovery check"

transport="$(wolf-b auto --policy secure --json | jq -r '.transport')"

if [[ "$transport" == "quic" ]]; then
  pass "QUIC recovered"
else
  fail "QUIC recovery failed; selected: $transport"
fi

echo
echo "🧠 Policy sanity"
wolf-b policy-test

echo
if [[ "$failures" == "0" ]]; then
  echo "🎉 CHAOS GATE PASSED"
  exit 0
else
  echo "⚠ CHAOS GATE FAILED ($failures)"
  exit 1
fi
