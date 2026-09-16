#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Script Gate"
echo "======================"
echo

failures=0

for script in scripts/*.sh; do
  echo "🧪 $script"

  if bash -n "$script"; then
    echo "✅ syntax ok"
  else
    echo "❌ syntax failed"
    failures=$((failures + 1))
  fi

  if [[ -x "$script" ]]; then
    echo "✅ executable"
  else
    echo "❌ not executable"
    failures=$((failures + 1))
  fi

  echo
done

if [[ "$failures" == "0" ]]; then
  echo "🎉 SCRIPT GATE PASSED"
  exit 0
else
  echo "⚠ SCRIPT GATE FAILED ($failures)"
  exit 1
fi
