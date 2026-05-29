#!/usr/bin/env bash
set -e

VERSION="${1:-v0.1.0-rc1}"

echo "🐺 WerewolfProxy Final RC"
echo "Version: $VERSION"
echo

echo "🧱 Running release check..."
./scripts/release-check.sh

echo
echo "📦 Creating final source checkpoint..."
mkdir -p ../releases

ARCHIVE="../releases/werewolfproxy_${VERSION}_$(date +%Y%m%d_%H%M%S).tar.gz"

tar \
  --exclude='target' \
  --exclude='.git' \
  --exclude='logs' \
  --exclude='*.tar.gz' \
  -czvf "$ARCHIVE" .

echo
echo "✅ Final RC archive created:"
echo "   $ARCHIVE"

echo
echo "🏷️ Suggested git tag:"
echo "   git tag -a $VERSION -m \"WerewolfProxy $VERSION\""
echo "   git push origin $VERSION"

echo
echo "🐺 RC complete."
