#!/usr/bin/env bash
set -euo pipefail

interval="${1:-30}"
project_dir="$(cd "$(dirname "$0")/.." && pwd)"

mkdir -p ~/.config/systemd/user

cat > ~/.config/systemd/user/werewolf-peer-monitor.service <<SERVICE
[Unit]
Description=Werewolf Peer Reputation Monitor

[Service]
Type=simple
WorkingDirectory=$project_dir
ExecStart=$project_dir/scripts/peer-monitor.sh $interval
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
SERVICE

systemctl --user daemon-reload
systemctl --user enable --now werewolf-peer-monitor.service

echo "✅ Werewolf peer monitor installed"
systemctl --user status werewolf-peer-monitor.service --no-pager
