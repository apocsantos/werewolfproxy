#!/usr/bin/env bash
set -euo pipefail

systemctl --user disable --now werewolf-b-watchdog.timer 2>/dev/null || true
systemctl --user stop werewolf-b-watchdog.service 2>/dev/null || true

rm -f ~/.config/systemd/user/werewolf-b-watchdog.timer
rm -f ~/.config/systemd/user/werewolf-b-watchdog.service

systemctl --user daemon-reload

echo "✅ Werewolf B watchdog timer removed"
