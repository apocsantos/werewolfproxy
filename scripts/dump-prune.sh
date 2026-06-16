#!/usr/bin/env bash
set -euo pipefail

keep="${1:-20}"
dump_dir="${WEREWOLF_DUMP_DIR:-$HOME/.cache/werewolf/dumps}"

mkdir -p "$dump_dir"

echo "🐺 Werewolf Dump Prune"
echo "====================="
echo "keeping newest: $keep"
echo

mapfile -t old_dirs < <(ls -1td "$dump_dir"/runtime-* 2>/dev/null | tail -n +$((keep + 1)))

if [[ "${#old_dirs[@]}" == "0" ]]; then
  echo "✅ nothing to prune"
  exit 0
fi

for d in "${old_dirs[@]}"; do
  echo "🗑 removing $(basename "$d")"
  rm -rf "$d"
done

echo
echo "✅ pruned ${#old_dirs[@]} dump(s)"
