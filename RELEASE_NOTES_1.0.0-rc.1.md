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
