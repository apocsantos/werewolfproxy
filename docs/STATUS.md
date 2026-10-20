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
| [D09](STAGE22R_D09_STORAGE_FAULT_CERTIFICATION.md) | Storage faults fail closed (full, unwritable, read-only, torn state) | Certified on `b911e41`; indeterminate commit unit-tested only |

The HC02 and LC-01 fixes are on `main` after the RC1 tag, so they are not in
the 1.0.0-rc.1 archive.

## Known gaps

See [known limitations](../KNOWN_LIMITATIONS.md). Main items:

- Only Linux x86_64 is certified.
- No anonymity, cover traffic or timing obfuscation.
- No automatic peer pairing; public keys are exchanged out of band.
- Stage20 did not run netem loss/jitter, ENOSPC, read-only filesystem,
  clock-change or systemd-restart chaos campaigns.
- The shared transport trait, health scoring and automatic return to the
  preferred transport are not implemented.

## History

The pre-RC "operational beta" status page, including the script-era
transport policies and roadmap, is kept at
[history/STATUS_PRE_RC.md](history/STATUS_PRE_RC.md).
