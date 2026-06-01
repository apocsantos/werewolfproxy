#!/usr/bin/env bash
set -euo pipefail

ROOT="${ROOT:-$HOME/Projects/werewolfproxy}"
BIG_DIR="${BIG_DIR:-/tmp/werewolf-bigtest}"
BIG_FILE="$BIG_DIR/large.bin"
SIZE_MB="${SIZE_MB:-50}"

mkdir -p "$BIG_DIR"

if [[ ! -f "$BIG_FILE" ]]; then
  echo "📦 Creating ${SIZE_MB}MiB large test file..."
  dd if=/dev/urandom of="$BIG_FILE" bs=1M count="$SIZE_MB" status=progress
fi

start_server() {
  local dir="$1"
  local port="$2"
  local name="$3"

  if curl -fsSI --max-time 2 "http://127.0.0.1:${port}/" >/dev/null 2>&1; then
    echo "✅ $name already running on $port"
    return 0
  fi

  echo "🚀 Starting $name on $port"
  nohup python3 -m http.server "$port" --directory "$dir" \
    >"/tmp/werewolf-${name}-${port}.log" 2>&1 &

  sleep 1

  curl -fsSI --max-time 5 "http://127.0.0.1:${port}/" >/dev/null \
    && echo "✅ $name ready on $port" \
    || { echo "❌ $name failed on $port"; exit 1; }
}

start_server "$ROOT" 8080 "target"
start_server "$BIG_DIR" 8081 "large-target"

echo
echo "Targets ready:"
echo "  http://127.0.0.1:8080/"
echo "  http://127.0.0.1:8081/large.bin"
