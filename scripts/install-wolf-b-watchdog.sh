#!/usr/bin/env bash
set -euo pipefail

mkdir -p ~/.config/systemd/user

cat > ~/.config/systemd/user/werewolf-b-watchdog.service <<SERVICE
[Unit]
Description=WerewolfProxy Wolf B Transport Watchdog

[Service]
Type=oneshot
ExecStart=/bin/bash -lc "%h/.local/bin/wolf-b heal --quiet || true"
SERVICE

cat > ~/.config/systemd/user/werewolf-b-watchdog.timer <<TIMER
[Unit]
Description=Run WerewolfProxy Wolf B Transport Watchdog

[Timer]
OnBootSec=30
OnUnitActiveSec=30
AccuracySec=5

[Install]
WantedBy=timers.target
TIMER

systemctl --user daemon-reload
systemctl --user enable --now werewolf-b-watchdog.timer

echo "✅ Werewolf B watchdog timer installed"
systemctl --user list-timers --no-pager | grep werewolf-b-watchdog || true
