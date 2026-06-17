#!/usr/bin/env bash
set -euo pipefail

mkdir -p .git/hooks

cat > .git/hooks/pre-push <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail

echo "🐺 Werewolf pre-push gate"
echo "========================"
echo

echo "🧱 Build"
cargo build >/dev/null

echo
echo "🎯 Targets"
./scripts/test-targets.sh

echo
echo "🔧 Heal"
wolf-b heal --quiet || true

echo
echo "🧪 Quick gate"
./scripts/quick-gate.sh

echo
echo "📦 JSON contracts"
./scripts/json-contract-gate.sh

echo
echo "🧬 Schema gate"
./scripts/schema-gate.sh

echo
echo "✅ pre-push checks passed"
HOOK

chmod +x .git/hooks/pre-push

echo "✅ git pre-push hook installed"
