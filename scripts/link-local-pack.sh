#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Linking Local Werewolf Pack"
echo "============================="
echo

fp_a="$(wolf-a pelt fingerprint | awk '/fingerprint:/ {print $2}')"
fp_b="$(wolf-b pelt fingerprint | awk '/fingerprint:/ {print $2}')"
fp_c="$(wolf-c pelt fingerprint | awk '/fingerprint:/ {print $2}')"
fp_d="$(wolf-d pelt fingerprint | awk '/fingerprint:/ {print $2}')"

add_peer() {
  local wolf="$1"
  local name="$2"
  local fp="$3"
  local addr="$4"

  echo "$wolf -> $name $addr"

  "$wolf" pack add "$name" "$fp" "$addr" 2>/dev/null || true
  "$wolf" pack set-address "$name" "$addr" 2>/dev/null || true
}

add_peer wolf-a wolf-b "$fp_b" "quic://127.0.0.1:9561"
add_peer wolf-a wolf-c "$fp_c" "quic://127.0.0.1:9562"
add_peer wolf-a wolf-d "$fp_d" "quic://127.0.0.1:9563"

add_peer wolf-b wolf-a "$fp_a" "quic://127.0.0.1:9560"
add_peer wolf-b wolf-c "$fp_c" "quic://127.0.0.1:9562"
add_peer wolf-b wolf-d "$fp_d" "quic://127.0.0.1:9563"

add_peer wolf-c wolf-a "$fp_a" "quic://127.0.0.1:9560"
add_peer wolf-c wolf-b "$fp_b" "quic://127.0.0.1:9561"
add_peer wolf-c wolf-d "$fp_d" "quic://127.0.0.1:9563"

add_peer wolf-d wolf-a "$fp_a" "quic://127.0.0.1:9560"
add_peer wolf-d wolf-b "$fp_b" "quic://127.0.0.1:9561"
add_peer wolf-d wolf-c "$fp_c" "quic://127.0.0.1:9562"

echo
echo "✅ local pack linked"
