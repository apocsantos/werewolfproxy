#!/usr/bin/env bash
: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required for local control}"
set -e

echo "🛑 Stopping Werewolf services..."
systemctl --user stop werewolf-a werewolf-b || true

echo "🧹 Killing leftover werewolfd processes..."
pkill -f "werewolfd --socket ${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-a/control.sock" || true
pkill -f "werewolfd --socket ${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-b/control.sock" || true


echo "⏳ Waiting for Werewolf ports to clear..."

for i in {1..30}; do
    if ! ss -ltnp | grep -qE '127\.0\.0\.1:(8443|9443|9020)' &&
       ! ss -lunp | grep -qE '127\.0\.0\.1:(9560|9561)'; then
        echo "✅ Ports released"
        echo "🐺 Starting Werewolf services..."
        systemctl --user start werewolf-a werewolf-b
        sleep 2
        echo "✅ Werewolf services restarted"
        systemctl --user --no-pager --full status werewolf-a werewolf-b
        exit 0
    fi
    sleep 0.5
done

echo "❌ Ports still busy:"
ss -ltnp | grep -E '8443|9443|9020' || true
ss -lunp | grep -E '9560|9561' || true
exit 1
