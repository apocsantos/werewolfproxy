#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Session End"
echo "======================"
echo

echo "📸 Snapshot"
wolf-b snapshot || true

echo
echo "📊 Final status"
wolf-b status-plus || true

echo
echo "🩺 Final doctor"
wolf-b doctor || true

echo
echo "📂 Git"
git status --short || true

echo
echo "💾 Suggested:"
echo "git add ."
echo 'git commit -m "wip: session checkpoint"'
echo "git push"

echo
echo "🌙 Wolf sleeping"
