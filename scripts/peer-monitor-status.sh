#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Peer Monitor Status"
echo "=============================="
echo

systemctl --user status werewolf-peer-monitor.service --no-pager || true

echo
echo "📜 Recent logs"
journalctl --user -u werewolf-peer-monitor.service -n 40 --no-pager || true
