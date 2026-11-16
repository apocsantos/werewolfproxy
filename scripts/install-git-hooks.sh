#!/usr/bin/env bash
set -euo pipefail

# Resolve from the installer, including when invoked from another directory.
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
HOOK=$(git rev-parse --git-path hooks/pre-push)
mkdir -p -- "$(dirname -- "$HOOK")"
PENDING=$(mktemp "${HOOK}.tmp.XXXXXX")
trap 'rm -f -- "$PENDING"' EXIT

cat > "$PENDING" <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail

# Resolve the pushing worktree at run time; hooks can be shared by worktrees.
ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"
echo "Werewolf pre-push: local Python unit tests and advisory audit"
exec ./scripts/ci-local.sh --fast
HOOK

chmod +x "$PENDING"
if [ -f "$HOOK" ] && cmp -s -- "$PENDING" "$HOOK"; then
  chmod +x "$HOOK"
  echo "git pre-push hook already current: $HOOK"
  exit 0
fi

if [ -e "$HOOK" ] || [ -L "$HOOK" ]; then
  BACKUP=$(mktemp "${HOOK}.backup.XXXXXX")
  cp -p -- "$HOOK" "$BACKUP"
  echo "Previous pre-push hook saved: $BACKUP"
fi
mv -f -- "$PENDING" "$HOOK"

echo "git pre-push hook installed: $HOOK"
