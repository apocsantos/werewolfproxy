# WerewolfProxy Operations

## Local targets

```bash
./scripts/test-targets.sh
Daily development
./scripts/quick-gate.sh
Release candidate validation
./scripts/release-gate.sh
Full validation
./scripts/full-gate.sh
Chaos validation
./scripts/chaos-gate.sh
./scripts/chaos-gate-extended.sh
Transport health
wolf-b health-summary
wolf-b doctor
wolf-b ready
wolf-b status-plus
JSON contracts
wolf-b doctor --json
wolf-b ready --json
wolf-b selftest --json
wolf-b status-json
wolf-b cache-status --json
Policies
wolf-b auto --policy secure
wolf-b auto --policy performance
wolf-b auto --policy stealth
wolf-b auto --policy resilience
wolf-b auto --policy learned
wolf-b auto --policy recent
Scoring and learning
wolf-b score
wolf-b score-save
wolf-b score-history
wolf-b policy-test
wolf-b policy-explain secure
Snapshots and drift
wolf-b snapshot
wolf-b snapshot-diff
wolf-b snapshot-alert 20
Maintenance
wolf-b maintenance
wolf-b cache-prune
wolf-b cache-status
Watchdog
./scripts/install-wolf-b-watchdog.sh
wolf-b watchdog-status
./scripts/uninstall-wolf-b-watchdog.sh
Validation ladder

Recommended order:

quick-gate     -> fast local development
json-contract  -> JSON/API contract validation
rc-gate        -> release candidate health
release-gate   -> release checkpoint
full-gate      -> full heavy validation
chaos gates    -> destructive resilience validation

