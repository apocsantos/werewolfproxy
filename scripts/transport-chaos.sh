#!/usr/bin/env bash
: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required for local control}"
set -euo pipefail

echo "🐺 Werewolf Chaos Test"
echo "======================"
echo

echo "📊 Initial state"
wolf-b health-summary
echo

echo "🔥 Step 1: kill QUIC Fang"
wolf-b fang list | grep -B5 "transport: quic" | grep "fang_" | awk '{print $2}' | while read -r fang; do
  echo "closing: $fang"
  werewolfctl --socket "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-b/control.sock" fang close "$fang" || true
done

sleep 2

echo
echo "🩺 State after QUIC removal"
wolf-b auto
echo

echo "⏳ Waiting for watchdog recovery..."
sleep 35

echo
echo "🩺 State after watchdog"
wolf-b health-summary

echo
echo "✅ Chaos test complete"
