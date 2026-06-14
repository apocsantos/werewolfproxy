#!/usr/bin/env bash
set -euo pipefail

mkdir -p ~/.config/systemd/user

cat > ~/.config/systemd/user/werewolf-b-maintenance.service <<SERVICE
[Unit]
Description=WerewolfProxy Wolf B Maintenance

[Service]
Type=oneshot
ExecStart=%h/.local/bin/wolf-b maintenance
SERVICE

cat > ~/.config/systemd/user/werewolf-b-maintenance.timer <<TIMER
[Unit]
Description=Run WerewolfProxy Wolf B Maintenance

[Timer]
OnBootSec=2min
OnCalendar=daily
Persistent=true
AccuracySec=30min

[Install]
WantedBy=timers.target
TIMER

systemctl --user daemon-reload
systemctl --user enable --now werewolf-b-maintenance.timer

echo "✅ Werewolf B maintenance timer installed"
systemctl --user list-timers --no-pager | grep werewolf-b-maintenance || true
