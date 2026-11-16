# WerewolfProxy Daily Commands

Developer workflow for this repository. Operating a node is in
[OPERATIONS](OPERATIONS.md), part 1.

## Start of session

```bash
cd ~/Projects/werewolfproxy
git status --short --branch
```

## Validation

```bash
./scripts/install-git-hooks.sh  # install/upgrade this checkout's pre-push hook
./scripts/ci-local.sh --fast    # Python unit tests and cargo audit; also runs on push
./scripts/ci-local.sh           # acceptance and ten disposable lab gates as well
```

The installer respects `core.hooksPath` and linked worktrees, and saves a
changed existing hook as `pre-push.backup.*` alongside it. Reinstalling an
identical hook creates no extra backup. Existing clones must rerun the
installer to replace the old live-node hook; pulling alone does not
update installed hooks. The hook runs the pushing worktree's local fast
checks and blocks the push if they fail. It does not build Rust or
replace the full pre-merge checks. `cargo-audit` must be installed, and
its advisory database refresh can require network access.

Local CI uses disposable fixtures and writes logs and `summary.tsv` to
`target/ci-local/<UTC>/`. Do not edit the tree during the full suite:
acceptance checks source preservation. No installed-node command is
part of routine checkout validation or the push hook.

Workspace checks before a commit:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --locked --offline --workspace
git diff --check
```

Stage lab gates use disposable daemons; how to run each, and what it
asserts, is in [tests/integration/README.md](../tests/integration/README.md).
For example:

```bash
python3 -B tests/integration/sync_gate.py
python3 -B tests/integration/auto_sync_gate.py
```

## Installed-node operations (manual)

The commands below use installed profiles/services or legacy live-node
gates. Run them only when intentionally operating those nodes; they are
separate from local CI and never invoked by the pre-push hook.

```bash
./scripts/test-targets.sh
./scripts/systemd-restart.sh
wolf-b heal
wolf-b status-plus
./scripts/quick-gate.sh
./scripts/release-gate.sh
./scripts/full-gate.sh
./scripts/nightly-gate.sh 30 10
```

## Diagnose

```bash
wolf-b doctor
wolf-b doctor --json | jq .
wolf-b selftest
wolf-b policy-test
wolf-b score-history
```

## Recovery

```bash
wolf-b doctor-fix
wolf-b heal
./scripts/systemd-restart.sh
```

## Reports

```bash
./scripts/gate-report.sh quick
wolf-b report-list
wolf-b report-show
wolf-b report-alert
```

## Stop point

```bash
git status
wolf-b snapshot
wolf-b status-plus
```
