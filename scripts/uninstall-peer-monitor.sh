#!/usr/bin/env bash
set -euo pipefail

systemctl --user disable --now werewolf-peer-monitor.service 2>/dev/null || true
rm -f ~/.config/systemd/user/werewolf-peer-monitor.service
systemctl --user daemon-reload

echo "✅ Werewolf peer monitor uninstalled"
