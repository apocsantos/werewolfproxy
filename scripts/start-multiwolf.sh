#!/usr/bin/env bash
: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required for local control}"
set -euo pipefail

mkdir -p -m 0700 ~/.config/wolf-a ~/.config/wolf-b ~/.config/wolf-c ~/.config/wolf-d

echo "🐺 Starting Werewolf local pack"
echo "=============================="
echo

start_wolf() {
  local name="$1"
  local socket="$2"
  local home="$3"
  local tcp_port="$4"
  local quic_port="$5"

  echo "starting $name..."

  pkill -f "werewolfd --socket $socket" 2>/dev/null || true

  nohup werewolfd \
    --socket "$socket" \
    --home "$home" \
    --listen "127.0.0.1:$tcp_port" \
    --quic-listen "127.0.0.1:$quic_port" \
    > "/tmp/${name}.log" 2>&1 &

  sleep 1
}

start_wolf wolf-a "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-a/control.sock" "$HOME/.config/wolf-a" 8443 9560
start_wolf wolf-b "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-b/control.sock" "$HOME/.config/wolf-b" 9443 9561
start_wolf wolf-c "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-c/control.sock" "$HOME/.config/wolf-c" 10443 9562
start_wolf wolf-d "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-d/control.sock" "$HOME/.config/wolf-d" 11443 9563

echo
echo "✅ local pack started"
echo
echo "Sockets:"
echo "  wolf-a ${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-a/control.sock"
echo "  wolf-b ${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-b/control.sock"
echo "  wolf-c ${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-c/control.sock"
echo "  wolf-d ${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-d/control.sock"
