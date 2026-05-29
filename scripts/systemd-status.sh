#!/usr/bin/env bash
set -e

echo "🐺 werewolf-a"
systemctl --user status werewolf-a --no-pager || true

echo
echo "🐺 werewolf-b"
systemctl --user status werewolf-b --no-pager || true
