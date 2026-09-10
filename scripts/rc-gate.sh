#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.local/bin:$PATH"

WOLFA="./scripts/wolf-a.sh"
WOLFB="./scripts/wolf-b.sh"

TARGET_URL="${WEREWOLF_TARGET_URL:-http://127.0.0.1:8080}"
QUIC_URL="${WEREWOLF_QUIC_URL:-http://127.0.0.1:9020}"
TCP_URL="${WEREWOLF_TCP_URL:-http://127.0.0.1:9021}"
TCP_ENC_URL="${WEREWOLF_TCP_ENC_URL:-http://127.0.0.1:9022}"

pass() { echo "✅ $1"; }
fail() { echo "❌ $1"; exit 1; }
warn() { echo "⚠️ $1"; }

restore_quic_on_exit() {
  if ! curl --max-time 3 -fsS "${QUIC_URL:-http://127.0.0.1:9020}" >/dev/null 2>&1; then
    echo
    echo "🔁 Cleanup: restoring QUIC Fang..."
    "$WOLFB" fang open-profile home-web-quic >/dev/null 2>&1 || true
    sleep 1
  fi
}

trap restore_quic_on_exit EXIT


echo "🐺 WerewolfProxy RC Gate"
echo "========================"
echo

echo "🧱 Build"
cargo build >/dev/null
pass "cargo build"

echo
echo "🧰 Services"
systemctl --user is-active --quiet werewolf-a && pass "werewolf-a active" || fail "werewolf-a not active"
systemctl --user is-active --quiet werewolf-b && pass "werewolf-b active" || fail "werewolf-b not active"

echo
echo "🧰 Control sockets"
[[ -S /tmp/wolf-a.sock ]] && pass "wolf-a socket present" || fail "wolf-a socket missing"
[[ -S /tmp/wolf-b.sock ]] && pass "wolf-b socket present" || fail "wolf-b socket missing"

echo
echo "🌐 Target"
curl --max-time 5 -fsS "$TARGET_URL" >/dev/null && pass "target reachable" || fail "target failed"

echo
echo "🐺 Wolf A"
"$WOLFA" health >/tmp/wolf-a-health.txt
grep -q "overall: HEALTHY" /tmp/wolf-a-health.txt && pass "wolf-a healthy" || { cat /tmp/wolf-a-health.txt; fail "wolf-a unhealthy"; }

"$WOLFA" transport-check >/tmp/wolf-a-transport.txt
grep -q "target:       reachable" /tmp/wolf-a-transport.txt && pass "wolf-a target reachable" || { cat /tmp/wolf-a-transport.txt; fail "wolf-a transport failed"; }
grep -q "socket:       present" /tmp/wolf-a-transport.txt && pass "wolf-a socket present" || { cat /tmp/wolf-a-transport.txt; fail "wolf-a socket failed"; }

echo
echo "🐺 Wolf B"
"$WOLFB" health >/tmp/wolf-b-health.txt
grep -q "overall: HEALTHY" /tmp/wolf-b-health.txt && pass "wolf-b healthy" || { cat /tmp/wolf-b-health.txt; fail "wolf-b unhealthy"; }

echo
echo "🦷 Active Fangs"
"$WOLFB" fang list >/tmp/wolf-b-fangs.txt
cat /tmp/wolf-b-fangs.txt

grep -q "transport: quic" /tmp/wolf-b-fangs.txt && pass "QUIC Fang active" || fail "QUIC Fang not active"

if grep -q "transport: tcp-plain" /tmp/wolf-b-fangs.txt; then
  pass "TCP fallback active"
else
  warn "TCP fallback not active; opening home-web-tcp"
  "$WOLFB" fang open-profile home-web-tcp >/dev/null || fail "could not open TCP fallback"
  sleep 1
fi

echo
echo "🚇 Tunnel checks"
curl --max-time 8 -fsS "$QUIC_URL" >/dev/null && pass "QUIC tunnel healthy" || fail "QUIC tunnel failed"
curl --max-time 8 -fsS "$TCP_URL" >/dev/null && pass "TCP fallback healthy" || fail "TCP fallback failed"

echo
echo "🤖 Auto selector"
"$WOLFB" auto >/tmp/wolf-b-auto.txt
cat /tmp/wolf-b-auto.txt
grep -q "transport: QUIC" /tmp/wolf-b-auto.txt && pass "auto selects QUIC" || fail "auto did not select QUIC"

"$WOLFB" auto --json >/tmp/wolf-b-auto-json.txt
cat /tmp/wolf-b-auto-json.txt | jq .
jq -e '.transport == "quic" and .healthy == true' /tmp/wolf-b-auto-json.txt >/dev/null \
  && pass "auto json selects QUIC" || fail "auto json failed"

"$WOLFB" score-json >/tmp/wolf-b-score-json.txt
cat /tmp/wolf-b-score-json.txt | jq .
jq -e '.transports.quic.score != null and .transports["tcp-encrypted-v2"].score != null and .transports["tcp-plain"].score != null' /tmp/wolf-b-score-json.txt >/dev/null \
  && pass "transport scores available" || fail "transport scores missing"

"$WOLFB" auto --policy legacy --json >/tmp/wolf-b-auto-legacy-json.txt
cat /tmp/wolf-b-auto-legacy-json.txt | jq .
jq -e '.healthy == true and (.transport == "quic" or .transport == "tcp-encrypted-v2" or .transport == "tcp-plain")' /tmp/wolf-b-auto-legacy-json.txt >/dev/null \
  && pass "legacy policy selects healthy transport" || fail "legacy policy failed"

"$WOLFB" auto --policy strict --json >/tmp/wolf-b-auto-strict-json.txt
cat /tmp/wolf-b-auto-strict-json.txt | jq .
jq -e '.transport == "quic" and .healthy == true and .security_downgrade == false' /tmp/wolf-b-auto-strict-json.txt >/dev/null \
  && pass "strict policy selects QUIC" || fail "strict policy failed"

echo
echo "🟡 Failover simulation"
"$WOLFB" fang list >/tmp/wolf-b-fangs.txt
quic_id="$(awk '/- fang_/ {id=$2} /transport: quic/ {print id}' /tmp/wolf-b-fangs.txt | head -1)"
[[ -n "${quic_id:-}" ]] || fail "could not identify QUIC Fang ID"

"$WOLFB" fang close "$quic_id" >/dev/null
sleep 1

"$WOLFB" auto >/tmp/wolf-b-auto-fallback.txt
cat /tmp/wolf-b-auto-fallback.txt
grep -q "transport: TCP encrypted v2" /tmp/wolf-b-auto-fallback.txt && pass "auto falls back to TCP encrypted v2" || fail "auto fallback failed"

echo
echo "🔁 Restore QUIC"
"$WOLFB" fang open-profile home-web-quic >/dev/null
sleep 1

"$WOLFB" auto >/tmp/wolf-b-auto-restored.txt
cat /tmp/wolf-b-auto-restored.txt
grep -q "transport: QUIC" /tmp/wolf-b-auto-restored.txt && pass "auto returns to QUIC" || fail "auto did not return to QUIC"

echo
echo "🩺 Doctor"
"$WOLFB" doctor && pass "wolf-b doctor healthy" || fail "wolf-b doctor failed"

"$WOLFB" doctor --json >/tmp/wolf-b-doctor-json.txt
cat /tmp/wolf-b-doctor-json.txt | jq .
jq -e '.healthy == true and .failures == 0 and (.checks | length) > 0' /tmp/wolf-b-doctor-json.txt >/dev/null \
  && pass "wolf-b doctor json healthy" || fail "wolf-b doctor json failed"

"$WOLFB" ready --json >/tmp/wolf-b-ready-json.txt
cat /tmp/wolf-b-ready-json.txt | jq .
jq -e '.ready == true and .doctor.healthy == true and .selected.healthy == true' /tmp/wolf-b-ready-json.txt >/dev/null \
  && pass "wolf-b ready json healthy" || fail "wolf-b ready json failed"

"$WOLFB" selftest --json >/tmp/wolf-b-selftest-json.txt
cat /tmp/wolf-b-selftest-json.txt | jq .
jq -e '.ok == true and .doctor.healthy == true and .ready.ready == true' /tmp/wolf-b-selftest-json.txt >/dev/null \
  && pass "wolf-b selftest json healthy" || fail "wolf-b selftest json failed"

echo
echo "📊 Benchmark"
"$WOLFB" benchmark

echo
echo "🎉 RC GATE PASSED"
