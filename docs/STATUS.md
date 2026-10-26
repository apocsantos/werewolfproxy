# WerewolfProxy Status

## Release

**1.0.0-rc.1**: release candidate for Linux x86_64
(`x86_64-unknown-linux-gnu`, certified on Debian 13.6 / glibc 2.41).
The tag `v1.0.0-rc.1` and its artifacts are frozen by the
[Stage21 audit](STAGE21_RC1_AUDIT_FREEZE.md), with the Stage20S rustls
remediation (rustls 0.23.45, rustls-webpki 0.103.15).

## Certified

| Area | Records |
|---|---|
| Target authorization | Stage9 |
| Replay hardening | Stage10 |
| Transport, local and session security | Stage11A–C |
| Receiver authentication and wire opacity | Stage12 |
| Traffic morphology (documented, not hidden) | Stage13 |
| Cryptographic hygiene | Stage14 |
| Persistent-state anti-rollback and manifest | Stage15, Stage15B |
| Pre-auth resource bounds | Stage16 |
| Linux service lifecycle, CLI, packaging | Stage17–19 |
| Adversarial soak | Stage20 |
| RUSTSEC-2026-0285 remediation | Stage20S |
| RC1 audit and freeze | Stage21 |

Supported transports are QUIC and encrypted TCP; plain TCP is an explicit
opt-in only. See [transport architecture](../TRANSPORT_ARCHITECTURE.md).

## Post-RC1 validation (Stage22 / Stage22R)

Stage22 ran on one host against the exact RC1 archive. Stage22R ran over the
real WAN between an initiator and a remote Debian VPS receiver:

| Record | Scope | Result |
|---|---|---|
| [Stage22](STAGE22_PRIVATE_OPERATIONAL_VALIDATION.md) | Private operational validation of the exact RC1 archive | Certified; host reboot not tested |
| LC-01 | Fail closed on Pack endpoint change; await Fang quiescence | Fixed on `main` |
| [HC02](STAGE22R_HC02_LOCAL_CANDIDATE.md) | Encrypted TCP full-duplex cancellation safety | Fixed, WAN certified |
| [QUIC WAN](STAGE22R_QUIC_WAN_CERTIFICATION.md) | Sustained full-duplex QUIC over WAN | Certified |
| [D01](STAGE22R_D01_RECEIVER_RESTART_CERTIFICATION.md) | Receiver restart | Certified |
| [D02](STAGE22R_D02_INITIATOR_RESTART_CERTIFICATION.md) | Initiator restart | Certified |
| [D03](STAGE22R_D03_HARD_CRASH_CERTIFICATION.md) | Initiator hard crash (SIGKILL) | Certified |
| [D04](STAGE22R_D04_RECEIVER_HARD_CRASH_CERTIFICATION.md) | Receiver hard crash (SIGKILL) | Certified |
| [D05A](STAGE22R_D05A_HARD_PATH_LOSS_CERTIFICATION.md) | Hard WAN path loss | Certified |
| [D05B](STAGE22R_D05B_TEMPORARY_BLACKHOLE_CERTIFICATION.md) | Temporary WAN blackhole, same-session survival | Certified |
| [D06](STAGE22R_D06_REPEATED_LOSS_RECOVERY_CERTIFICATION.md) | Repeated WAN blackholes and hard path loss without restart | Certified on `b911e41` |
| [D07](STAGE22R_D07_TARGET_FAILURE_CERTIFICATION.md) | Target failure mid-session (RST, crash, half-close, absent) | Certified on `b911e41` |
| [D08](STAGE22R_D08_RESOURCE_PRESSURE_CERTIFICATION.md) | Resource pressure at production ceilings (sessions, stalled TCP/QUIC, RPC rate, slow targets) | Certified on `bcf7bbd` |
| [D09](STAGE22R_D09_STORAGE_FAULT_CERTIFICATION.md) | Storage faults fail closed (full, unwritable, read-only, torn state) | Certified on `b911e41`; indeterminate commit unit-tested only |

The HC02 and LC-01 fixes are on `main` after the RC1 tag, so they are not in
the 1.0.0-rc.1 archive.

## Development on `main` after RC1 (Stage23–26)

None of this is in the 1.0.0-rc.1 archive.

| Stage | Scope | State |
|---|---|---|
| Stage23 | Named services: `service add/list/show/remove/connect`, `services.json` | Implemented; lab gate `service_gate.py` |
| [Stage24](STAGE24_FINDNODE_DESIGN.md) | Pack-only peer discovery: `ping` and `find_node` RPCs, signed contact records, multi-hop lookup | Implemented; lab gate `discovery_gate.py`; WAN checked |
| [Stage25](STAGE25_SERVICE_DISCOVERY_DESIGN.md) | Service discovery: `service publish`, `discover services`, `service add --from`; visibility follows Target Policy | Implemented; lab gate `service_discovery_gate.py`; WAN checked |
| [Stage26](STAGE26_MULTIHOP_ROUTING_DESIGN.md) | Multi-hop routing through one relay, end-to-end identity and authorization; `route set/list/remove` | Implemented; lab gate `routed_fang_gate.py`; WAN checked (three defects found and fixed) |
| [Stage27](STAGE27_PROVIDER_SELECTION_DESIGN.md) | Provider selection: ordered providers per service, per-connection failover before establishment only | Implemented; lab gate `provider_gate.py`; WAN checked |
| [Stage28](STAGE28_GOVERNANCE_SURVEY.md) | Network governance: 7-of-10 signed statements (genesis, key rotation, network revocation), offline signing tool, chain head in the protected manifest (schema 2), fork fencing | Implemented; lab gate `governance_gate.py`; no network component to WAN-check (statements are operator-delivered) |
| [Stage29](STAGE29_ACTIVATION_DESIGN.md) | Admission and activation: network roster (at most 1024), one-way activation as a chain fact, locally derived admission with a fence for unadmitted nodes, `pack add` bounded by the roster, permanent revocation; manual `governance sync` from Pack peers with conflict fencing | Implemented; lab gates `activation_gate.py`, `sync_gate.py`; WAN checked (sync through a relay, disposable daemons only) |
| [Stage30](STAGE30_LIFECYCLE_SURVEY.md) | Network lifecycle: chain, node and capability states; DEGRADED vs UNRECOVERABLE criteria; 7-of-10 death statement and local retirement; destruction, crash semantics and the Tombstone | Decisions accepted 2026-10-26; [spec](STAGE30_LIFECYCLE_SPEC.md) for review (docs only, no code yet) |

## Known gaps

See [known limitations](../KNOWN_LIMITATIONS.md). Main items:

- Only Linux x86_64 is certified.
- No anonymity, cover traffic or timing obfuscation; a Stage26 relay sees
  who talks to which listener, when and how much.
- No automatic peer pairing; public keys are exchanged out of band.
- Not run anywhere: netem-style random loss, jitter and reordering, and
  host clock changes. Restart, crash, path-loss, resource-pressure and
  storage-fault behaviour was exercised on later builds by Stage22R
  (D01–D09), not on the rc.1 archive.
- Routing has one operator-set relay per destination, with no route
  discovery or automatic selection. Path loss between the requester and the
  relay was exercised on both hops; between the relay and the destination
  it was not.
- The shared transport trait, health scoring and automatic return to the
  preferred transport are not implemented.

## History

The pre-RC "operational beta" status page, including the script-era
transport policies and roadmap, is kept at
[history/STATUS_PRE_RC.md](history/STATUS_PRE_RC.md).
