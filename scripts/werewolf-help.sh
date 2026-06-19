#!/usr/bin/env bash
set -euo pipefail

cat <<'HELP'
🐺 Werewolf Developer Help
==========================

Core service:
  werewolf-start-a
  werewolf-start-b
  wolf-a status
  wolf-b status

Transport:
  wolf-b health-summary
  wolf-b transport-health-json
  wolf-b benchmark
  wolf-b benchmark --json
  wolf-b auto
  wolf-b auto --json
  wolf-b auto --policy secure
  wolf-b auto --policy performance
  wolf-b auto --policy stealth
  wolf-b auto --policy resilience
  wolf-b auto --policy learned
  wolf-b auto --policy recent

Scoring and memory:
  wolf-b score
  wolf-b score-json
  wolf-b score-save
  wolf-b score-history
  wolf-b score-prune

Policies:
  wolf-b policy-test
  wolf-b policy-explain secure
  wolf-b policy-explain recent

Healing and diagnostics:
  wolf-b heal
  wolf-b heal --quiet
  wolf-b doctor
  wolf-b doctor --json
  wolf-b doctor-fix
  wolf-b ready
  wolf-b ready --json
  wolf-b selftest
  wolf-b selftest --json

Snapshots and cache:
  wolf-b snapshot
  wolf-b snapshot-diff
  wolf-b snapshot-alert 20
  wolf-b snapshot-prune
  wolf-b maintenance
  ./scripts/maintenance-all.sh
  ./scripts/wolf-dashboard.sh
  ./scripts/wolf-dashboard-json.sh
  ./scripts/wolf-dashboard-watch.sh 5
  wolf-b maintenance-report
  wolf-b report-list
  wolf-b report-show
  wolf-b report-diff
  wolf-b report-alert
  wolf-b cache-status
  wolf-b cache-status --json
  wolf-b cache-prune
  wolf-b status-json
  wolf-b status-plus
  wolf-b export-status

Gates:
  ./scripts/gate-index.sh
  ./scripts/quick-gate.sh
  ./scripts/json-contract-gate.sh
  ./scripts/rc-gate.sh
  ./scripts/dev-gate.sh
  ./scripts/chaos-gate.sh
  ./scripts/chaos-gate-extended.sh
  ./scripts/soak-gate.sh 5 5
  ./scripts/soak-gate.sh 5 5 --chaos
  ./scripts/multiwolf-gate.sh
  ./scripts/full-gate.sh
  ./scripts/release-gate.sh

Targets:
  ./scripts/test-targets.sh

Runtime dumps:
  ./scripts/runtime-dump.sh
  ./scripts/dump-list.sh
  ./scripts/dump-prune.sh 20

Watchdog:
  ./scripts/install-wolf-b-watchdog.sh
  ./scripts/uninstall-wolf-b-watchdog.sh
  wolf-b watchdog-status
HELP


Peer:
  ./scripts/peer-db.sh list
  ./scripts/peer-db.sh ping-all
  ./scripts/peer-db.sh status
  ./scripts/peer-db.sh best
  ./scripts/peer-db.sh export
  ./scripts/peer-contract.sh
  ./scripts/peer-gate.sh
  ./scripts/peer-monitor.sh 30
  ./scripts/install-peer-monitor.sh 30
  ./scripts/peer-monitor-status.sh
  ./scripts/uninstall-peer-monitor.sh
