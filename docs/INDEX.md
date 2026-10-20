# Documentation index

## Start here

- [README](../README.md)
- [Two-node quickstart](../QUICKSTART.md)
- [Install and upgrade guide](../packaging/INSTALL.md)
- [Security reporting](../SECURITY.md)
- [Known limitations](../KNOWN_LIMITATIONS.md)
- [Release notes](../RELEASE_NOTES_1.0.0-rc.1.md)
- [Current status](STATUS.md)
- [Architecture and final objective](ARCHITECTURE.md)
- [Transport architecture](../TRANSPORT_ARCHITECTURE.md)
- [Session-end signalling proposal](SESSION_END_SIGNALLING_PROPOSAL.md) (approved; gaps 1–3 fixed)
- [Stage24 FindNode design](STAGE24_FINDNODE_DESIGN.md) (Phase 1 implemented), [RPC wire spec](STAGE24_FINDNODE_RPC_WIRE_SPEC.md) (approved) and [discover CLI](V1_CLI_CONTRACT.md#stage24-extension-discovery)
- [Stage25 service discovery design](STAGE25_SERVICE_DISCOVERY_DESIGN.md) (design accepted) and [`services` RPC wire spec](STAGE25_SERVICES_RPC_WIRE_SPEC.md) (implemented)

## Frozen v1 contracts

- [Threat model](V1_THREAT_MODEL.md)
- [Security properties](V1_SECURITY_PROPERTIES.md)
- [Wire contract](V1_WIRE_CONTRACT.md)
- [Persistence contract](V1_PERSISTENCE_CONTRACT.md)
- [CLI contract](V1_CLI_CONTRACT.md)
- [Linux service contract](V1_LINUX_SERVICE.md)
- [Cryptographic inventory](V1_CRYPTOGRAPHY.md)

## Certification records

- [Stage20S RUSTSEC-2026-0285 remediation](STAGE20S_RUSTLS_2026_0285_REMEDIATION.md)
- [Stage21 RC1 audit and freeze](STAGE21_RC1_AUDIT_FREEZE.md)

The current Stage21 release uses the Stage20S-certified dependency graph:
rustls 0.23.45 and rustls-webpki 0.103.15. Earlier stage reports retain the
dependency versions and lockfile digests used at their original certification
dates; they are historical records, not the current resolved release graph.

Stage 9–20 reports are retained under docs/STAGE*.md. The final RC audit,
dependency/license review, release artifact hashes and tag record are in
[Stage21 audit and freeze](STAGE21_RC1_AUDIT_FREEZE.md).

## Post-RC1 operational validation

- [Stage22 private operational validation](STAGE22_PRIVATE_OPERATIONAL_VALIDATION.md)
- [Stage22R HC02 encrypted TCP full-duplex](STAGE22R_HC02_LOCAL_CANDIDATE.md)
- [Stage22R QUIC real-WAN](STAGE22R_QUIC_WAN_CERTIFICATION.md)
- [Stage22R D01 receiver restart](STAGE22R_D01_RECEIVER_RESTART_CERTIFICATION.md)
- [Stage22R D02 initiator restart](STAGE22R_D02_INITIATOR_RESTART_CERTIFICATION.md)
- [Stage22R D03 initiator hard crash](STAGE22R_D03_HARD_CRASH_CERTIFICATION.md)
- [Stage22R D04 receiver hard crash](STAGE22R_D04_RECEIVER_HARD_CRASH_CERTIFICATION.md)
- [Stage22R D05A hard path loss](STAGE22R_D05A_HARD_PATH_LOSS_CERTIFICATION.md)
- [Stage22R D05B temporary blackhole](STAGE22R_D05B_TEMPORARY_BLACKHOLE_CERTIFICATION.md)
- [Stage22R D06 repeated loss and recovery](STAGE22R_D06_REPEATED_LOSS_RECOVERY_CERTIFICATION.md)
- [Stage22R D07 target failure](STAGE22R_D07_TARGET_FAILURE_CERTIFICATION.md)
- [Stage22R D09 storage faults](STAGE22R_D09_STORAGE_FAULT_CERTIFICATION.md)

## History

- [Pre-RC status snapshot](history/STATUS_PRE_RC.md)
- [Archived legacy scripts and support bundles](../archive/README.md)
