#!/usr/bin/env bash
set -e

systemctl --user disable werewolf-a >/dev/null 2>&1 || true
systemctl --user disable werewolf-b >/dev/null 2>&1 || true

systemctl --user stop werewolf-a >/dev/null 2>&1 || true
systemctl --user stop werewolf-b >/dev/null 2>&1 || true

rm -f "$HOME/.config/systemd/user/werewolf-a.service"
rm -f "$HOME/.config/systemd/user/werewolf-b.service"

systemctl --user daemon-reload

echo "✅ Werewolf systemd user services removed"
