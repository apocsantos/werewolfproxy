#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Extended Chaos Gate"
echo "=============================="
echo

failures=0

pass() {
  echo "✅ $1"
}

fail() {
  echo "❌ $1"
  failures=$((failures + 1))
}

close_port() {
  local port="$1"
  local label="$2"

  fang_id="$(
    wolf-b fang list \
      | awk -v port="$port" '
        /- fang_/ {id=$2}
        $0 ~ ("local:  127.0.0.1:" port) {
          print id
          exit
        }
      '
  )"

  if [[ -z "${fang_id:-}" ]]; then
    fail "could not find $label Fang on port $port"
    return 1
  fi

  echo "closing $label Fang on $port: $fang_id"
  wolf-b fang close "$fang_id" >/dev/null || true
  sleep 2
}

echo "🔧 Pre-heal"
wolf-b heal --quiet || true
sleep 2

echo
echo "🩺 Baseline"

wolf-b selftest --json | jq -e '.ok == true' >/dev/null \
  && pass "baseline healthy" \
  || fail "baseline unhealthy"

echo
echo "⚡ Kill QUIC"
close_port 9020 "QUIC"

transport="$( { wolf-b auto --policy secure --json || [[ "$?" == 2 ]]; } | jq -r '.transport')"

[[ "$transport" == "tcp-encrypted-v2" ]] \
  && pass "fallback to tcp-encrypted-v2" \
  || fail "unexpected fallback: $transport"

echo
echo "⚡ Kill TCP encrypted v2"
close_port 9022 "TCP encrypted v2"

transport="$( { wolf-b auto --policy secure --json || [[ "$?" == 2 ]]; } | jq -r '.transport')"

[[ "$transport" == "unavailable" ]] \
  && pass "strict exhaustion fails closed" \
  || fail "unexpected fallback: $transport"

echo
for policy in compatibility legacy; do
  plain_args=()
  [[ "$policy" != compatibility ]] || plain_args=(--allow-plain-fallback)
  wolf-b auto --policy "$policy" "${plain_args[@]}" --json | jq -e \
    '.transport == "tcp-plain" and .security_downgrade == true' >/dev/null \
    && pass "$policy explicit weaker operation" || fail "$policy plain selection"
done

echo "🔧 Healing all"
wolf-b heal --quiet || true
sleep 4

echo
echo "🧪 Recovery"

transport="$( { wolf-b auto --policy secure --json || [[ "$?" == 2 ]]; } | jq -r '.transport')"

[[ "$transport" == "quic" ]] \
  && pass "full recovery to QUIC" \
  || fail "did not recover to QUIC"

echo
echo "🧠 Policy sanity"
wolf-b policy-test

echo
if [[ "$failures" == "0" ]]; then
  echo "🎉 EXTENDED CHAOS PASSED"
  exit 0
else
  echo "⚠ EXTENDED CHAOS FAILED ($failures)"
  exit 1
fi
