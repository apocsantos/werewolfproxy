#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/.."

rm -f /tmp/wolf-a.sock

cargo run -p werewolfd -- \
  --socket /tmp/wolf-a.sock \
  --home ~/.config/wolf-a \
  --listen 127.0.0.1:8443 \
  --quic-listen 127.0.0.1:9560
