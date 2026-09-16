#!/usr/bin/env bash
: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required for local control}"
set -e
cd "$(dirname "$0")/.."


cargo run -p werewolfd -- \
  --socket "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-b/control.sock" \
  --home ~/.config/wolf-b \
  --listen 127.0.0.1:9443 \
  --quic-listen 127.0.0.1:9561
