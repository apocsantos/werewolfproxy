#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf Repo Gate"
echo "===================="
echo

failures=0

fail() {
  echo "❌ $1"
  failures=$((failures + 1))
}

pass() {
  echo "✅ $1"
}

echo "📂 Git branch"
git branch --show-current

echo
echo "📦 Git status"
git status --short

echo
echo "🔎 Checking accidental editor/temp files"

bad_files="$(
  find . \
    -path './.git' -prune -o \
    -type f \( \
      -name '*~' -o \
      -name '*.swp' -o \
      -name '*.tmp' -o \
      -name 'kate' \
    \) -print
)"

if [[ -n "$bad_files" ]]; then
  echo "$bad_files"
  fail "temporary/editor files found"
else
  pass "no temporary/editor files"
fi

echo
echo "🔐 Script permissions"

if find scripts -name '*.sh' ! -perm -u+x | grep .; then
  fail "non-executable scripts found"
else
  pass "all scripts executable"
fi

echo
if [[ "$failures" == "0" ]]; then
  echo "🎉 REPO GATE PASSED"
  exit 0
else
  echo "⚠ REPO GATE FAILED ($failures)"
  exit 1
fi
