#!/usr/bin/env bash
: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required for local control}"
set -e
cd "$(dirname "$0")/.."


cargo run -p werewolfd -- \
  --socket "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-a/control.sock" \
  --home ~/.config/wolf-a \
  --listen 127.0.0.1:8443 \
  --quic-listen 127.0.0.1:9560
