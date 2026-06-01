#!/usr/bin/env bash
set -euo pipefail

SIZE_MB="${1:-50}"
SRC_DIR="/tmp/werewolf-bigtest"
SRC_FILE="$SRC_DIR/large.bin"
DST_FILE="/tmp/werewolf-tcp-v2-test.bin"
URL="${WEREWOLF_TCP_V2_LARGE_URL:-http://127.0.0.1:9032/large.bin}"

mkdir -p "$SRC_DIR"

if [[ ! -f "$SRC_FILE" ]]; then
  echo "📦 Creating ${SIZE_MB}MiB payload..."
  dd if=/dev/urandom of="$SRC_FILE" bs=1M count="$SIZE_MB" status=progress
fi

echo "🌐 Checking direct source..."
curl -fsSI --max-time 5 "http://127.0.0.1:8081/large.bin" >/dev/null || {
  echo "❌ Source server unavailable on 127.0.0.1:8081"
  echo "Run in another terminal:"
  echo "  cd $SRC_DIR && python3 -m http.server 8081"
  exit 1
}

echo "🔐 Downloading through TCP encrypted v2..."
rm -f "$DST_FILE"
curl --fail --max-time 180 -o "$DST_FILE" "$URL"

echo "🔎 Verifying SHA256..."
src_hash="$(sha256sum "$SRC_FILE" | awk '{print $1}')"
dst_hash="$(sha256sum "$DST_FILE" | awk '{print $1}')"

echo "source: $src_hash"
echo "tunnel: $dst_hash"

if [[ "$src_hash" != "$dst_hash" ]]; then
  echo "❌ HASH MISMATCH"
  exit 1
fi

echo "✅ TCP encrypted v2 large transfer passed (${SIZE_MB}MiB)"
