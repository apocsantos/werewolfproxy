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

echo
echo "🟡 Failover simulation"
"$WOLFB" fang list >/tmp/wolf-b-fangs.txt
quic_id="$(awk '/- fang_/ {id=$2} /transport: quic/ {print id}' /tmp/wolf-b-fangs.txt | head -1)"
[[ -n "${quic_id:-}" ]] || fail "could not identify QUIC Fang ID"

"$WOLFB" fang close "$quic_id" >/dev/null
sleep 1

"$WOLFB" auto >/tmp/wolf-b-auto-fallback.txt
cat /tmp/wolf-b-auto-fallback.txt
grep -q "transport: TCP fallback" /tmp/wolf-b-auto-fallback.txt && pass "auto falls back to TCP" || fail "auto fallback failed"

echo
echo "🔁 Restore QUIC"
"$WOLFB" fang open-profile home-web-quic >/dev/null
sleep 1

"$WOLFB" auto >/tmp/wolf-b-auto-restored.txt
cat /tmp/wolf-b-auto-restored.txt
grep -q "transport: QUIC" /tmp/wolf-b-auto-restored.txt && pass "auto returns to QUIC" || fail "auto did not return to QUIC"

echo
echo "📊 Benchmark"
"$WOLFB" benchmark

echo
echo "🎉 RC GATE PASSED"
