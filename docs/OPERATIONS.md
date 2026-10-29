# WerewolfProxy Operations

Part 1 is for operating a node with `werewolfctl`; the exact rules are in
the [CLI contract](V1_CLI_CONTRACT.md). Part 2 is the development harness
of this repository (`wolf-b`, `scripts/`), which is not part of the
release.

## Part 1. Operating a node

### Health

```bash
werewolfctl status
werewolfctl doctor
werewolfctl --json status
werewolfctl governance status
```

`status` reports the node's lifecycle (`DYING` while a death or retirement
runs); `governance status` reports the chain head, roster, admission,
fork flag and the automatic sync (`auto_sync`: eligibility, phase, next
run, last run).

### Trust and forwarding

Pack, Target Policy, Fangs and Silver are as in 1.0.0-rc.1 (see
[QUICKSTART](../QUICKSTART.md)). Revoking or removing a peer fences it at
runtime before the change is saved. `silver on` fences all new forwarding;
`silver off` only after the storage condition is healthy.

### Services

```bash
werewolfctl service add web node-b 127.0.0.1:8080 http
werewolfctl service connect web --local-port 18080
werewolfctl service provider add web node-c 127.0.0.1:8080
werewolfctl service publish web 127.0.0.1:8080 http
werewolfctl service published
werewolfctl discover services
werewolfctl service add web --from node-b
```

A service is intent; every connection is still authorized by the
provider's Target Policy. Providers are tried in order before a session is
established, never after. A published service is visible only to Pack
members its Target Policy grants.

### Discovery

```bash
werewolfctl discover advertise quic://203.0.113.5:9560 tcp://203.0.113.5:8443
werewolfctl discover refresh
werewolfctl discover contacts
werewolfctl discover find PUBLIC_KEY_B64
```

Discovery finds Pack members' addresses and never adds trust. A node
whose wall clock is more than 7 days off cannot publish a contact record
its peers accept.

### Routes

```bash
werewolfctl route set node-c --via node-b --hop quic
werewolfctl route list
werewolfctl route remove node-c
```

The relay (`node-b`) must grant this node `node-c`'s encrypted-TCP
listener address with `target allow`. Route changes apply to Fangs opened
afterwards; changing, revoking or removing the relay closes Fangs through
it.

### Governance

Key holders work offline with `governance keygen`, `draft`, `sign`,
`combine`, `inspect` and `check`. On each node:

```bash
werewolfctl governance apply genesis.wwgov --expect-hash HASH
werewolfctl governance apply statement.wwgov
werewolfctl governance sync
werewolfctl governance auto-sync off
werewolfctl governance auto-sync on
```

- A genesis needs a manifest-enabled Den (`state manifest-migrate`) and the
  hash exchanged out of band; it moves the manifest to schema 2 and cannot
  be undone. Back up the Den before it.
- After activation, `pack add` accepts only roster keys, and a node whose
  own key is not admitted is fenced.
- Automatic sync is on by default on active, admitted nodes and runs every
  7.5–22.5 minutes; manual `sync` and discovery commands take precedence.
- A fork (`GOVERNANCE_FORK`) fences governance and is never resolved
  automatically.

### Death and retirement

```bash
werewolfctl governance apply death.wwgov
werewolfctl den retire --confirm OWN_FINGERPRINT
```

Both are terminal: the node destroys its identity key, then its Den,
leaves a Tombstone and exits 67, and refuses to start again (the systemd
unit does not restart it). A death statement arrives only as a file.
There is no claim about storage media, logs, swap or backups.

### Backup and upgrade

Follow [INSTALL](../packaging/INSTALL.md): stop the service and copy the
whole Den, permissions preserved, before an upgrade or a genesis. A
governed Den cannot be used by 1.0.0-rc.1 binaries.

## Part 2. Development harness

### Local targets

```bash
./scripts/test-targets.sh
```

### Gates

| Gate | Command |
|---|---|
| Daily development | `./scripts/quick-gate.sh` |
| JSON/API contracts | `./scripts/json-contract-gate.sh` |
| Release candidate health | `./scripts/rc-gate.sh` |
| Release checkpoint | `./scripts/release-gate.sh` |
| Full validation | `./scripts/full-gate.sh` |
| Chaos | `./scripts/chaos-gate.sh`, `./scripts/chaos-gate-extended.sh` |

Recommended order: quick-gate, json-contract, rc-gate, release-gate,
full-gate, then the chaos gates (destructive resilience validation).

The post-rc.1 stages each have a disposable lab gate in
`tests/integration/` (`service_gate.py`, `discovery_gate.py`,
`service_discovery_gate.py`, `routed_fang_gate.py`, `provider_gate.py`,
`governance_gate.py`, `activation_gate.py`, `sync_gate.py`,
`death_gate.py`, `auto_sync_gate.py`), and the Stage32 campaigns run with
`tests/integration/chaos.py` in an unprivileged sandbox. None of them
touches the long-running development wolves. How to run each is in
[tests/integration/README.md](../tests/integration/README.md).

### Transport health

```bash
wolf-b health-summary
wolf-b doctor
wolf-b ready
wolf-b status-plus
```

### JSON contracts

```bash
wolf-b doctor --json
wolf-b ready --json
wolf-b selftest --json
wolf-b status-json
wolf-b cache-status --json
```

Reference JSON contracts live in `contracts/` and are checked by
`./scripts/schema-gate.sh`. If a schema changed intentionally, regenerate
them with `./scripts/update-contracts.sh` and review `git diff --
contracts`. A schema-gate failure is either a regression or an intentional
API change; never update contracts without checking the diff.

### Policies

```bash
wolf-b auto --policy secure
wolf-b auto --policy performance
wolf-b auto --policy stealth
wolf-b auto --policy resilience
wolf-b auto --policy learned
wolf-b auto --policy recent
```

### Scoring, snapshots and drift

```bash
wolf-b score
wolf-b score-save
wolf-b score-history
wolf-b policy-test
wolf-b policy-explain secure
wolf-b snapshot
wolf-b snapshot-diff
wolf-b snapshot-alert 20
```

### Reports and regression alerts

```bash
wolf-b report-list
wolf-b report-show
wolf-b report-diff
wolf-b report-alert
```

### Maintenance, watchdog and dumps

```bash
wolf-b maintenance
wolf-b cache-prune
wolf-b cache-status
./scripts/install-wolf-b-watchdog.sh
wolf-b watchdog-status
./scripts/uninstall-wolf-b-watchdog.sh
./scripts/runtime-dump.sh
./scripts/dump-list.sh
./scripts/dump-prune.sh 20
./scripts/maintenance-all.sh
./scripts/werewolf-help.sh
```
