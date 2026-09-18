# WerewolfProxy 1.0.0-rc.1

Status: Linux x86_64 release candidate. This is not final 1.0.0 stable.

## Scope

The candidate packages the certified TCP encrypted and QUIC secure forwarding
implementation, exact-SPKI receiver authentication, Ed25519 peer identity,
Stage9 exact target authorization, Stage11C runtime authority/Silver fencing,
Stage15B local protected-state consistency, Stage16 resource ceilings,
Stage17 Linux service lifecycle, and Stage18 CLI provisioning. Stage20 ran
recorded loopback reconnect, restart, mutation, resource recovery and transfer
campaigns.

Installation and operation target Linux x86_64 / x86_64-unknown-linux-gnu.
The tested environment is Debian 13.6 and glibc 2.41. The archive contains
werewolfd, werewolfctl, the systemd unit, install/uninstall scripts, operator
guides, checksums, dependency inventory and third-party notices.

## Security remediation before publication

The Stage21 prepublication cargo-audit gate detected RUSTSEC-2026-0285 in
rustls 0.23.40. Stage20S separately certified the update to rustls 0.23.45
and rustls-webpki 0.103.15. The final candidate uses the patched dependency
graph; cargo audit reported zero vulnerabilities and zero warnings against
the 2026-09-18 advisory database. The fix rejects a malformed TLS 1.3
encryption-level transition and does not change Werewolf application framing,
persistence, or CLI behavior. Technical evidence is in the
[Stage20S remediation certification](docs/STAGE20S_RUSTLS_2026_0285_REMEDIATION.md).

## Security boundaries

Public-key trust is configured out of band. Pack membership is not unlimited
target authorization. Exact target grants and bounded session/resource policy
remain separate. A complete historical internally consistent Den can still
be restored. Traffic metadata, distributed denial of service, root compromise,
other platforms, and the unexecuted Stage20 chaos categories are listed in
[Known limitations](KNOWN_LIMITATIONS.md).

Back up the complete Den before upgrade. Software uninstall retains Den and
Pelt. Do not downgrade persistence state after a future incompatible migration.
See the [operator docs index](docs/INDEX.md).
