#!/usr/bin/env bash
set -euo pipefail

dump_dir="${WEREWOLF_DUMP_DIR:-$HOME/.cache/werewolf/dumps}"
mkdir -p "$dump_dir"

ts="$(date +%Y%m%d_%H%M%S)"
out="$dump_dir/runtime-$ts"

mkdir -p "$out"

echo "🐺 Werewolf Runtime Dump"
echo "======================="
echo "dir: $out"
echo

echo "🧩 Process list"
ps aux > "$out/ps.txt"

echo "🔌 Listening ports"
ss -tulpn > "$out/ports.txt" || true

echo "🐺 wolf-b status"
wolf-b status-json > "$out/status.json" || true

echo "🩺 doctor"
wolf-b doctor --json > "$out/doctor.json" || true

echo "🧪 selftest"
wolf-b selftest --json > "$out/selftest.json" || true

echo "🦷 fang list"
wolf-b fang list > "$out/fangs.txt" || true

echo "📊 score"
wolf-b score-json > "$out/scores.json" || true

echo "📸 latest report"
wolf-b report-show latest > "$out/latest-report.json" || true

echo "📜 service logs"
journalctl --user -u werewolf-a -n 200 --no-pager > "$out/wolf-a.log" || true
journalctl --user -u werewolf-b -n 200 --no-pager > "$out/wolf-b.log" || true

echo
echo "✅ runtime dump created: $out"
