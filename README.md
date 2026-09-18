# WerewolfProxy 1.0.0-rc.1

**Release candidate for Linux x86_64.** This release provides authenticated
TCP and QUIC forwarding between explicitly trusted peers. Trust uses exchanged
public Ed25519 identity keys; each receiver separately authorizes exact target
addresses. It does not provide anonymity, traffic-flow confidentiality, or
immunity to denial of service.

The certified release target is x86_64-unknown-linux-gnu, tested on Debian
13.6 with glibc 2.41. Other platforms and libc baselines are not certified.
See [Known limitations](KNOWN_LIMITATIONS.md) and the [security model](docs/V1_THREAT_MODEL.md).

## Install and provision

Use the v1 RC archive and follow [INSTALL](packaging/INSTALL.md) and the
[two-node quickstart](QUICKSTART.md). System installation runs as root only
to install files and provision the service account; the daemon runs as the
unprivileged werewolf account. Identity creation and peer trust are explicit.

Basic supported administration uses werewolfctl: den init, pelt init/show,
pack add/list/remove, target allow/list/remove, fang create/list/activate/
deactivate/remove, silver status/on/off, status, doctor, and state migration
or backup-info. Public peer keys are exchanged out of band. Never exchange a
private key.

## Security boundary

The daemon authenticates the receiver through TLS 1.3 and exact-SPKI Pelt
binding, authenticates senders with signed connection-bound requests, applies
Pack membership and exact target authorization, and bounds remote-triggered
resource use. A restored complete historical Den can be accepted when all
local consistency records agree. Traffic timing, endpoints, packet sizes and
volume remain observable. A compromised privileged host can access secrets
while the daemon runs.

The prepublication RC audit's rustls advisory was remediated and separately
certified before this candidate was built. See the [release notes](RELEASE_NOTES_1.0.0-rc.1.md)
and [security certification records](docs/INDEX.md) for the security history.

WerewolfProxy 1.0.0-rc.1 is a release candidate, not the final stable release.
Operators are invited to test it on the certified platform and report issues
using the procedure in [SECURITY.md](SECURITY.md).

## Documentation

Start at the [documentation index](docs/INDEX.md) for the v1 contracts,
threat model, security property matrix, operator docs, limitations and
certification records.
