#!/usr/bin/env bash
set -euo pipefail

systemctl --user disable --now werewolf-b-maintenance.timer 2>/dev/null || true
systemctl --user stop werewolf-b-maintenance.service 2>/dev/null || true

rm -f ~/.config/systemd/user/werewolf-b-maintenance.timer
rm -f ~/.config/systemd/user/werewolf-b-maintenance.service

systemctl --user daemon-reload

echo "✅ Werewolf B maintenance timer removed"
