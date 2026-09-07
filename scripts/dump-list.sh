#!/usr/bin/env bash
set -euo pipefail

dump_dir="${WEREWOLF_DUMP_DIR:-$HOME/.cache/werewolf/dumps}"

echo "🐺 Werewolf Runtime Dumps"
echo "========================"
echo

if [[ ! -d "$dump_dir" ]]; then
  echo "no dumps found"
  exit 0
fi

ls -1td "$dump_dir"/runtime-* 2>/dev/null | head -n "${1:-10}" | while read -r d; do
  echo "$(basename "$d") | size=$(du -sh "$d" | awk '{print $1}')"
done
