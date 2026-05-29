#!/usr/bin/env bash
set -e

URL="${1:-http://127.0.0.1:9020}"
COUNT="${2:-30}"

echo "🌪️ Chaos curl test"
echo "URL:   $URL"
echo "Count: $COUNT"
echo

ok=0
fail=0

for i in $(seq 1 "$COUNT"); do
    if curl --max-time 5 -fsS "$URL" >/dev/null; then
        echo "[$i/$COUNT] ✅ ok"
        ok=$((ok+1))
    else
        echo "[$i/$COUNT] ❌ fail"
        fail=$((fail+1))
    fi

    sleep 1
done

echo
echo "Result:"
echo "  ok:   $ok"
echo "  fail: $fail"

if [ "$ok" -gt 0 ]; then
    echo "✅ Chaos curl completed"
else
    echo "❌ All requests failed"
    exit 1
fi
