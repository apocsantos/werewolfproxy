# WerewolfProxy Daily Commands

## Start of session

```bash
cd ~/Projects/werewolfproxy
./scripts/test-targets.sh
./scripts/systemd-restart.sh
wolf-b heal
wolf-b status-plus
Quick development validation
./scripts/quick-gate.sh
Before push
.git/hooks/pre-push
Before release checkpoint
./scripts/release-gate.sh
Heavy validation
./scripts/full-gate.sh
Long validation
./scripts/nightly-gate.sh 30 10
Diagnose
wolf-b doctor
wolf-b doctor --json | jq .
wolf-b selftest
wolf-b policy-test
wolf-b score-history
Recovery
wolf-b doctor-fix
wolf-b heal
./scripts/systemd-restart.sh
Reports
./scripts/gate-report.sh quick
wolf-b report-list
wolf-b report-show
wolf-b report-alert
Stop point
git status
wolf-b snapshot
wolf-b status-plus

EOF
