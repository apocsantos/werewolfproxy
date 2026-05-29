#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/.."

rm -f /tmp/wolf-b.sock

cargo run -p werewolfd -- \
  --socket /tmp/wolf-b.sock \
  --home ~/.config/wolf-b \
  --listen 127.0.0.1:9443 \
  --quic-listen 127.0.0.1:9561
