#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Dashboard Gate"
echo "========================="
echo

./scripts/wolf-dashboard-json.sh >/tmp/wolf-dashboard.json
cat /tmp/wolf-dashboard.json | jq .

jq -e '
  .doctor.healthy == true
  and .ready.ready == true
  and .status.ready.ready == true
  and .scores.transports.quic != null
  and .cache.scores != null
' /tmp/wolf-dashboard.json >/dev/null

echo
echo "🎉 DASHBOARD GATE PASSED"
