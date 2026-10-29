# WerewolfProxy 1.0.0-rc.2

**Release candidate for Linux x86_64, in preparation (Stage33).** The
frozen release is still [1.0.0-rc.1](RELEASE_NOTES_1.0.0-rc.1.md); rc.2 is
frozen only when its audit passes. See [STATUS](docs/STATUS.md).

WerewolfProxy provides authenticated TCP and QUIC forwarding between
explicitly trusted peers. Trust uses exchanged public Ed25519 identity keys;
each receiver separately authorizes exact target addresses. On top of that
it adds named services, discovery among trusted peers, one-relay routing,
threshold network governance and a terminal network lifecycle. It does not
provide anonymity, traffic-flow confidentiality, or immunity to denial of
service.

The certified release target is x86_64-unknown-linux-gnu, tested on Debian
13.6 with glibc 2.41. Other platforms and libc baselines are not certified.
See [Known limitations](KNOWN_LIMITATIONS.md) and the [security model](docs/V1_THREAT_MODEL.md).

## Install and provision

Use the release archive and follow [INSTALL](packaging/INSTALL.md) and the
[two-node quickstart](QUICKSTART.md). System installation runs as root only
to install files and provision the service account; the daemon runs as the
unprivileged werewolf account. Identity creation and peer trust are explicit.
Public peer keys are exchanged out of band. Never exchange a private key.

## Administration

Everything is administered with `werewolfctl`; the full contract, with
exit codes and `--json` output, is the [CLI contract](docs/V1_CLI_CONTRACT.md).

| Area | Commands |
|---|---|
| Identity and node | `den init/info/retire`, `pelt init/show`, `status`, `doctor`, `state manifest-migrate/backup-info` |
| Trust | `pack add/list/remove/revoke/set-address`, `target allow/list/remove`, `silver status/on/off` |
| Forwarding | `fang create/list/active/activate/deactivate/remove` |
| Services | `service add/list/show/remove/connect`, `service publish/unpublish/published`, `service provider add/remove` |
| Discovery | `discover advertise/withdraw/show/contacts/refresh/find/services` |
| Routing | `route set/list/remove` (one relay per destination) |
| Governance, on a node | `governance apply/status/sync`, `governance auto-sync on/off` |
| Governance, offline key holders | `governance keygen/draft/sign/combine/inspect/check` |

- **Services** name a peer and target; they are intent, not authority:
  every connection is still checked against Pack, Target Policy and
  Silver. Discovery shows a peer's published service only where its
  Target Policy would grant it.
- **Discovery** is Pack-only: it finds addresses and services of nodes
  already trusted and never adds trust.
- **Routing** goes through one operator-set relay. The relay learns who is
  talking to whom and the session's timing and volume, but not the
  service, the target or the payload.
- **Governance** statements need 7 of 10 governance keys: genesis, key
  rotation, network revocation, admission, activation and death. After
  activation the network roster bounds every node's Pack, and active,
  admitted nodes fetch new statements from their Pack peers automatically
  (on by default; `governance auto-sync off` turns it off). Applying
  a genesis is irreversible.
- **Death and retirement** are explicit and terminal. A death statement
  (7 of 10) or `den retire` on one node destroys the node's identity key
  and Den and leaves a minimal Tombstone. Death travels only as a file,
  never by sync.

## Security boundary

The daemon authenticates the receiver through TLS 1.3 and exact-SPKI Pelt
binding, authenticates senders with signed connection-bound requests, applies
Pack membership and exact target authorization, and bounds remote-triggered
resource use. Relays, discovery and governance never widen that: a relay
cannot authenticate as either end, discovery adds no trust, and governance
never adds a peer to a Pack: it bounds the Pack and revokes keys. A restored complete historical Den can be
accepted when all local consistency records agree. Traffic timing,
endpoints, packet sizes and volume remain observable. A compromised
privileged host can access secrets while the daemon runs, and OS root still
controls its own host's node. Destruction makes no claim about storage
media, logs, swap or backups.

Robustness was certified under packet loss, jitter, reordering,
duplication, corruption, rate limits, wall-clock jumps and process stalls
(Stage32); the limits it found are in [Known limitations](KNOWN_LIMITATIONS.md).

Issues are reported using the procedure in [SECURITY.md](SECURITY.md).

## Documentation

Start at the [documentation index](docs/INDEX.md) for the v1 contracts,
threat model, security property matrix, operator docs, limitations and
certification records, and at [STATUS](docs/STATUS.md) for what each stage
certified.
