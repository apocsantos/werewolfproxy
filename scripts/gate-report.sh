#!/usr/bin/env bash
set -euo pipefail

gate="${1:-quick}"
report_dir="${WEREWOLF_REPORT_DIR:-$HOME/.cache/werewolf/reports}"

mkdir -p "$report_dir"

ts="$(date +%Y%m%d_%H%M%S)"

json_report="$report_dir/gate-${gate}-${ts}.json"
md_report="$report_dir/gate-${gate}-${ts}.md"

echo "🐺 Werewolf Gate Report"
echo "======================="
echo "gate: $gate"
echo

run_gate() {
  case "$gate" in
    quick)
      ./scripts/quick-gate.sh
      ;;
    release)
      ./scripts/release-gate.sh
      ;;
    full)
      ./scripts/full-gate.sh
      ;;
    nightly)
      ./scripts/nightly-gate.sh 5 2
      ;;
    *)
      echo "unknown gate: $gate"
      exit 1
      ;;
  esac
}

status="passed"

if ! run_gate; then
  status="failed"
fi

jq -n \
  --arg timestamp "$(date -Iseconds)" \
  --arg gate "$gate" \
  --arg status "$status" \
  --argjson ready "$(wolf-b ready --json)" \
  --argjson doctor "$(wolf-b doctor --json)" \
  --argjson cache "$(wolf-b cache-status --json)" \
  --argjson status_json "$(wolf-b status-json)" \
  '{
    timestamp: $timestamp,
    gate: $gate,
    status: $status,
    ready: $ready,
    doctor: $doctor,
    cache: $cache,
    wolf_status: $status_json
  }' > "$json_report"

cat > "$md_report" <<MARKDOWN
# Werewolf Gate Report

- Timestamp: $(date -Iseconds)
- Gate: $gate
- Status: $status

## Ready

\`\`\`json
$(wolf-b ready --json | jq .)
\`\`\`

## Doctor

\`\`\`json
$(wolf-b doctor --json | jq .)
\`\`\`

## Status

\`\`\`json
$(wolf-b status-json | jq .)
\`\`\`
MARKDOWN

echo
echo "✅ JSON report: $json_report"
echo "✅ Markdown report: $md_report"

[[ "$status" == "passed" ]]
