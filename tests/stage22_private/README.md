# Stage22 private deployment harness

This harness deploys and tests only the certified Stage21 `v1.0.0-rc.1`
archive. It never builds a replacement candidate. It has two operating modes:

- `local_campaign.py` verifies the archive before extraction, uses the
  archive's installer with a temporary `DESTDIR`, and runs the Stage17,
  Stage18, Stage19, and Stage20 CI-safe harnesses against those staged
  binaries. Dens, identities, listeners, targets, and logs are disposable and
  local to the test run.
- `deploy.py` validates a private node configuration, stages the verified
  archive's binaries over SSH to each node's private runtime path, and can run
  `status`/`doctor` on the configured Den. It supports any two or more Linux
  nodes and explicit link topology such as A <-> B or A <-> B <-> C. It leaves
  identity creation, service activation, Pack exchange, grants, and Fang
  changes as explicit operator-controlled actions.

The default local campaign runs 10,000 TCP routes and 10,000 QUIC routes,
transfers 1 GiB over each transport, holds each session with periodic traffic
for 600 seconds, and then observes 300 seconds of idle operation. The local
runner exposes count and duration options for a harness smoke test; record
the selected values with every result.

## Private node configuration

Copy `nodes.example.json` to a local file outside the repository when
possible. If it is stored beside these scripts, `.gitignore` excludes the
provided `local.json` and `remote.json` names. Keep configuration mode 0600:

```sh
umask 077
cp tests/stage22_private/nodes.example.json "$HOME/stage22-nodes.json"
chmod 600 "$HOME/stage22-nodes.json"
```

Set real `host`, `ssh_destination`, Den/runtime paths, reachable listener
addresses, and target addresses only in that private copy. Do not add SSH
private keys, passwords, Pelt files/private keys, Pack secrets, tokens, or
credentials to the file. SSH authentication uses the operator's existing SSH
agent and SSH configuration. Generate independent Pelt identities on every
node; exchange only public provisioning values over the operator's trusted
channel. The example uses documentation-only addresses and hostnames.

The topology is an array of links by node name. A two-node link is
`[["node-a", "node-b"]]`; a three-node chain is
`[["node-a", "node-b"], ["node-b", "node-c"]]`. Each node has a distinct
Den and runtime directory. The `host` value is retained for the operator's
network map; SSH uses `ssh_destination`.

## Exact archive verification and local run

The archive argument must hash to the Stage21-certified SHA-256. The local
runner verifies that digest before extraction and verifies the archive's
inner `SHA256SUMS` before the temporary install. It rejects a different
archive; do not rebuild and relabel a candidate as the certified artifact.

```sh
umask 077
mkdir -p "$HOME/.cache/werewolf-stage22"
chmod 700 "$HOME/.cache/werewolf-stage22"
python3 -B tests/stage22_private/local_campaign.py \
  --archive /path/to/werewolfproxy-1.0.0-rc.1-linux-x86_64.tar.gz \
  --tmp-parent "$HOME/.cache/werewolf-stage22" \
  --report tests/stage22_private/results/local.json
```

The report contains hashes, counts, and bounded resource measurements; it
contains no private identities or payloads. Keep reports private. Local mode
uses isolated loopback Dens and does not reboot the host, change host routes,
modify systemd, or alter disk policy.

## Remote staging and health check

Run a topology-only validation first. It prints node aliases/roles but not
addresses:

```sh
python3 -B tests/stage22_private/deploy.py \
  --config "$HOME/stage22-nodes.json" --archive /path/to/certified.tar.gz --plan
```

After reviewing the private paths, stage the exact archive. This transfers
the archive over SSH, checks the pinned outer digest on each host, verifies
inner `SHA256SUMS`, then copies the two binaries into `<runtime_path>/bin`.
It does not start a process, create identity, enable systemd, or mutate Den
state.

```sh
python3 -B tests/stage22_private/deploy.py \
  --config "$HOME/stage22-nodes.json" --archive /path/to/certified.tar.gz --stage
```

After the operator has provisioned each node and started its daemon through
the chosen supported service or foreground procedure, check configured
`status` and `doctor`:

```sh
python3 -B tests/stage22_private/deploy.py \
  --config "$HOME/stage22-nodes.json" --archive /path/to/certified.tar.gz --check
```

For service installation and boot behavior, use the archive's `install.sh`,
packaged systemd unit, and `packaging/INSTALL.md` procedure after reviewing
the configured bind addresses and firewall. This harness deliberately does
not silently install a privileged host service.

## Operational procedure

Use the released `QUICKSTART.md` for pair provisioning and both secure
transports. For interruption, restart, backup/restore, target/Pack revocation,
Silver, and state validation, record only abstract node names and test
outcomes in the committed Stage22 report. Keep hostnames, IPs, usernames,
credential paths, and Pelt private material out of Git. Perform corruption,
disk-pressure, read-only, clock, reboot, and network-impairment tests only on
disposable systems or copies. Restore operator data before any destructive
drill.

## Limits

Local mode certifies loopback behavior only. It does not establish LAN, VLAN,
routed, VPN, WAN, NAT traversal, reboot, systemd boot, netem, clock-step,
ENOSPC, or read-only filesystem behavior. Remote deployment is configurable,
but an actual result requires operator-provided Linux nodes and SSH access.
Untested environments must remain explicitly `NOT TESTED` in the Stage22
report. Existing nonclaims on DoS, traffic-flow confidentiality, and complete
Den rollback freshness remain in force.
