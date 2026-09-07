#!/usr/bin/env bash
set -euo pipefail

mkdir -p ~/.config/wolf-a ~/.config/wolf-b ~/.config/wolf-c ~/.config/wolf-d

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
  rm -f "$socket"

  nohup werewolfd \
    --socket "$socket" \
    --home "$home" \
    --listen "127.0.0.1:$tcp_port" \
    --quic-listen "127.0.0.1:$quic_port" \
    > "/tmp/${name}.log" 2>&1 &

  sleep 1
}

start_wolf wolf-a /tmp/wolf-a.sock "$HOME/.config/wolf-a" 8443 9560
start_wolf wolf-b /tmp/wolf-b.sock "$HOME/.config/wolf-b" 9443 9561
start_wolf wolf-c /tmp/wolf-c.sock "$HOME/.config/wolf-c" 10443 9562
start_wolf wolf-d /tmp/wolf-d.sock "$HOME/.config/wolf-d" 11443 9563

echo
echo "✅ local pack started"
echo
echo "Sockets:"
echo "  wolf-a /tmp/wolf-a.sock"
echo "  wolf-b /tmp/wolf-b.sock"
echo "  wolf-c /tmp/wolf-c.sock"
echo "  wolf-d /tmp/wolf-d.sock"
