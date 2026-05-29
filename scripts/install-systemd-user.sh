#!/usr/bin/env bash
set -e

SYSTEMD_DIR="$HOME/.config/systemd/user"
BIN_DIR="$HOME/.local/bin"

mkdir -p "$SYSTEMD_DIR"

if ! command -v werewolfd >/dev/null 2>&1; then
  echo "❌ werewolfd not found in PATH"
  echo 'Run: export PATH="$HOME/.local/bin:$PATH"'
  exit 1
fi

cat > "$SYSTEMD_DIR/werewolf-a.service" <<EOS
[Unit]
Description=WerewolfProxy Wolf A
After=network-online.target

[Service]
Type=simple
ExecStartPre=/usr/bin/rm -f /tmp/wolf-a.sock
ExecStart=$BIN_DIR/werewolfd --socket /tmp/wolf-a.sock --home %h/.config/wolf-a --listen 127.0.0.1:8443 --quic-listen 127.0.0.1:9560
Restart=on-failure
RestartSec=2
KillMode=control-group
TimeoutStopSec=5

[Install]
WantedBy=default.target
EOS

cat > "$SYSTEMD_DIR/werewolf-b.service" <<EOS
[Unit]
Description=WerewolfProxy Wolf B
After=network-online.target

[Service]
Type=simple
ExecStartPre=/usr/bin/rm -f /tmp/wolf-b.sock
ExecStart=$BIN_DIR/werewolfd --socket /tmp/wolf-b.sock --home %h/.config/wolf-b --listen 127.0.0.1:9443 --quic-listen 127.0.0.1:9561
Restart=on-failure
RestartSec=2
KillMode=control-group
TimeoutStopSec=5

[Install]
WantedBy=default.target
EOS

systemctl --user daemon-reload

echo "✅ systemd user services installed:"
echo "  werewolf-a.service"
echo "  werewolf-b.service"
echo
echo "Start:"
echo "  systemctl --user start werewolf-a"
echo "  systemctl --user start werewolf-b"
echo
echo "Enable on login:"
echo "  systemctl --user enable werewolf-a"
echo "  systemctl --user enable werewolf-b"
