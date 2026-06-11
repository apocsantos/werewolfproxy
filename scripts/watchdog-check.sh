#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Watchdog Check"
echo "=========================="
echo

echo "🧹 Reinstalling watchdog..."
./scripts/uninstall-wolf-b-watchdog.sh >/dev/null 2>&1 || true
./scripts/install-wolf-b-watchdog.sh

echo
echo "⏳ Waiting 3 seconds..."
sleep 3

echo
echo "📊 Timer status"
systemctl --user status werewolf-b-watchdog.timer --no-pager || true

echo
echo "🩺 Running watchdog once"
systemctl --user start werewolf-b-watchdog.service

echo
echo "📋 Watchdog service"
systemctl --user status werewolf-b-watchdog.service --no-pager || true

echo
echo "🚇 Transport health"
wolf-b health-summary

echo
echo "✅ Watchdog check complete"
