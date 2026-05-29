#!/usr/bin/env bash
set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="$HOME/.local/bin"
CONFIG_A="$HOME/.config/wolf-a"
CONFIG_B="$HOME/.config/wolf-b"

echo "🐺 Installing Werewolf CLI..."

cd "$ROOT"

echo "🧱 Building release binaries..."
WEREWOLF_GIT_COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)" \
WEREWOLF_BUILD_DATE="$(date -u '+%Y-%m-%d %H:%M:%S UTC')" \
cargo build --release

echo "📁 Creating config directories..."
mkdir -p "$CONFIG_A" "$CONFIG_B" "$BIN_DIR"

echo "📦 Installing binaries..."
install -m 755 target/release/werewolfd "$BIN_DIR/werewolfd.new"
install -m 755 target/release/werewolfctl "$BIN_DIR/werewolfctl.new"
mv -f "$BIN_DIR/werewolfd.new" "$BIN_DIR/werewolfd"
mv -f "$BIN_DIR/werewolfctl.new" "$BIN_DIR/werewolfctl"

echo "🧰 Installing helper wrappers..."

if [[ -f "$ROOT/scripts/wolf-a.sh" ]]; then
  cp "$ROOT/scripts/wolf-a.sh" "$BIN_DIR/wolf-a"
else
  cat > "$BIN_DIR/wolf-a" <<'EOS'
#!/usr/bin/env bash
exec werewolfctl --socket /tmp/wolf-a.sock "$@"
EOS
fi

if [[ -f "$ROOT/scripts/wolf-b.sh" ]]; then
  cp "$ROOT/scripts/wolf-b.sh" "$BIN_DIR/wolf-b"
else
  cat > "$BIN_DIR/wolf-b" <<'EOS'
#!/usr/bin/env bash
exec werewolfctl --socket /tmp/wolf-b.sock "$@"
EOS
fi

cat > "$BIN_DIR/werewolf-start-a" <<'EOS'
#!/usr/bin/env bash
exec werewolfd \
  --socket /tmp/wolf-a.sock \
  --home ~/.config/wolf-a \
  --listen 127.0.0.1:8443 \
  --quic-listen 127.0.0.1:9560
EOS

cat > "$BIN_DIR/werewolf-start-b" <<'EOS'
#!/usr/bin/env bash
exec werewolfd \
  --socket /tmp/wolf-b.sock \
  --home ~/.config/wolf-b \
  --listen 127.0.0.1:9443 \
  --quic-listen 127.0.0.1:9561
EOS

chmod +x \
  "$BIN_DIR/werewolfd" \
  "$BIN_DIR/werewolfctl" \
  "$BIN_DIR/wolf-a" \
  "$BIN_DIR/wolf-b" \
  "$BIN_DIR/werewolf-start-a" \
  "$BIN_DIR/werewolf-start-b"

echo
echo "✅ Werewolf installed."
echo
echo "Make sure ~/.local/bin is in PATH:"
echo '  export PATH="$HOME/.local/bin:$PATH"'
echo
echo "Commands:"
echo "  werewolfd --help"
echo "  werewolfctl --help"
echo "  werewolf-start-a"
echo "  werewolf-start-b"
echo "  wolf-a status"
echo "  wolf-b status"
