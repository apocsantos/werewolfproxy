# Documentation index

## Start here

- [README](../README.md)
- [Two-node quickstart](../QUICKSTART.md)
- [Install and upgrade guide](../packaging/INSTALL.md)
- [Security reporting](../SECURITY.md)
- [Known limitations](../KNOWN_LIMITATIONS.md)
- [Release notes](../RELEASE_NOTES_1.0.0-rc.1.md)

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
