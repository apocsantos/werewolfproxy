#!/usr/bin/env bash
set -e

echo "🛑 stopping wolves..."

pkill -f "werewolfd.*wolf-a" || true
pkill -f "werewolfd.*wolf-b" || true

rm -f /tmp/wolf-a.sock /tmp/wolf-b.sock

echo "✅ Pack stopped"
