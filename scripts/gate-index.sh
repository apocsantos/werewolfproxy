#!/usr/bin/env bash
set -euo pipefail

cat <<'GATES'
🐺 Werewolf Gate Index
=====================

Fast:
  ./scripts/quick-gate.sh
    Fast local development gate.

Contracts:
  ./scripts/json-contract-gate.sh
    Validates runtime JSON outputs.
  ./scripts/schema-gate.sh
    Validates schema stability against contracts/.

Release:
  ./scripts/rc-gate.sh
    Main RC health gate.
  ./scripts/release-gate.sh
    Release checkpoint gate.

Development:
  ./scripts/dev-gate.sh
    Dev validation including RC and chaos.
  ./scripts/full-gate.sh
    Heavy validation with contracts, RC, chaos, soak.

Chaos:
  ./scripts/chaos-gate.sh
    Single QUIC failure and recovery.
  ./scripts/chaos-gate-extended.sh
    QUIC + TCP-v2 failure chain and recovery.

Soak:
  ./scripts/soak-gate.sh 5 5
    Repeated health/score/snapshot loop.
  ./scripts/soak-gate.sh 5 5 --chaos
    Soak with injected QUIC failure.

Nightly:
  ./scripts/nightly-gate.sh 30 10
    Longer resilience validation.

Reports:
  ./scripts/gate-report.sh quick
  ./scripts/gate-report.sh release
  ./scripts/gate-report.sh full
GATES
