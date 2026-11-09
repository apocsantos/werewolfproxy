#!/usr/bin/env bash
# Local CI (Stage40). Runs the checks that GitHub CI does not run.
#
#   scripts/ci-local.sh          full set: unit, audit, acceptance, ten lab gates (~6 min warm);
#                                do not edit the tree while it runs (acceptance checks source preservation)
#   scripts/ci-local.sh --fast   unit tests and cargo audit only (~5 s); used by the pre-push hook
#
# Checks run sequentially (the gates allocate loopback ports and must not overlap).
# Each check's log and a summary.tsv go to target/ci-local/<UTC timestamp>/.
# Exit status is nonzero if any check fails. Gates clean up only their own
# process groups; no live wolf is touched.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

MODE=full
case "${1:-}" in
  "") ;;
  --fast) MODE=fast ;;
  *) echo "usage: $0 [--fast]" >&2; exit 2 ;;
esac

if ! cargo audit --version >/dev/null 2>&1; then
  echo "ci-local: cargo-audit is not installed (cargo install cargo-audit --locked)" >&2
  exit 2
fi

OUT="target/ci-local/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$OUT"
{
  echo "commit $(git rev-parse HEAD)"
  git status --porcelain --untracked-files=no | sed 's/^/dirty /'
  echo "mode $MODE"
  rustc -V
  python3 -V
} > "$OUT/meta.txt"
printf 'check\texit\tseconds\n' > "$OUT/summary.tsv"

FAILED=()

check() { # name timeout_seconds cmd...
  local name=$1 limit=$2; shift 2
  local start end rc
  start=$(date +%s)
  printf '%-24s ' "$name"
  timeout "$limit" "$@" > "$OUT/$name.log" 2>&1
  rc=$?
  end=$(date +%s)
  printf '%s\t%s\t%s\n' "$name" "$rc" "$((end - start))" >> "$OUT/summary.tsv"
  if [ "$rc" -eq 0 ]; then
    echo "PASS ($((end - start)) s)"
  else
    echo "FAIL (exit $rc, $((end - start)) s, log $OUT/$name.log)"
    FAILED+=("$name")
  fi
}

check python-unit 300 python3 -B -m unittest discover -s tests/integration -p 'test_*.py'
# Ignored advisories live in .cargo/audit.toml; --deny warnings fails on any new
# unsound, unmaintained or yanked advisory.
check audit 600 cargo audit --deny warnings

if [ "$MODE" = full ]; then
  check acceptance 1800 python3 -B tests/integration/run.py
  for gate in activation auto_sync death discovery governance provider \
              routed_fang service service_discovery sync; do
    check "gate-$gate" 1800 python3 -B "tests/integration/${gate}_gate.py"
  done
fi

if [ "${#FAILED[@]}" -eq 0 ]; then
  echo "ci-local: PASS ($MODE) — $OUT"
  exit 0
fi
echo "ci-local: FAIL ($MODE): ${FAILED[*]} — $OUT" >&2
exit 1
