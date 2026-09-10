#!/usr/bin/env bash
set -euo pipefail

clear || true

echo "🐺 Werewolf Dashboard"
echo "===================="
echo

echo "🩺 Doctor"
wolf-b doctor --json 2>/dev/null | jq -r '
if .healthy then
  "healthy ✅"
else
  "degraded ❌ failures=\(.failures)"
end
' || echo "unknown"

echo
echo "🚦 Ready"
wolf-b ready --json 2>/dev/null | jq -r '
if .ready then
  "ready ✅"
else
  "not ready ❌"
end
' || echo "unknown"

echo
echo "🚇 Selected transport"
{ wolf-b auto --policy secure --json 2>/dev/null || [[ "$?" == 2 ]]; } | jq -r '
"\(.transport) | healthy=\(.healthy)"
' || echo "unknown"

echo
echo "📊 Scores"
wolf-b score 2>/dev/null || true

echo
echo "📸 Snapshot drift"
wolf-b snapshot-alert 20 || true

echo
echo "📝 Reports"
wolf-b report-list 3 || true

echo
echo "🦷 Fangs"
wolf-b fang list | tail -n 20 || true
