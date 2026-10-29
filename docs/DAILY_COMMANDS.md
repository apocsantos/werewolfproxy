# WerewolfProxy Daily Commands

Developer workflow for this repository. Operating a node is in
[OPERATIONS](OPERATIONS.md), part 1.

## Start of session

```bash
cd ~/Projects/werewolfproxy
./scripts/test-targets.sh
./scripts/systemd-restart.sh
wolf-b heal
wolf-b status-plus
```

## Validation

```bash
./scripts/quick-gate.sh          # quick development validation
.git/hooks/pre-push              # before push (runs on every push)
./scripts/release-gate.sh        # before a release checkpoint
./scripts/full-gate.sh           # heavy validation
./scripts/nightly-gate.sh 30 10  # long validation
```

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
python3 -B tests/integration/sync_gate.py [--keep-temp]
python3 -B tests/integration/auto_sync_gate.py [--keep-temp]
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
