# WerewolfProxy 1.0.0-rc.2

Status: Linux x86_64 release candidate, the architecture-complete candidate
of Stage33. This is not final 1.0.0 stable. It supersedes 1.0.0-rc.1 as the
candidate for 1.0.0; rc.1 stays frozen as released.

## Scope

Everything in 1.0.0-rc.1, plus the work on `main` since:

- **Named services** (Stage23): `service add/connect`, intent only.
- **Discovery** (Stage24, Stage25): Pack-only peer and service discovery
  with signed contact records; publication visible only where the
  provider's Target Policy grants it.
- **Routing** (Stage26): one operator-set relay per destination, with the
  inner session authorized end to end by the destination.
- **Provider failover** (Stage27): ordered providers per service, before
  establishment only.
- **Governance** (Stage28–29): 7-of-10 signed statements (genesis, key
  rotation, network revocation, admission, activation), a roster that
  bounds every Pack after activation, fork fencing, and `governance sync`.
- **Lifecycle** (Stage30): explicit death (7 of 10) and local retirement,
  destruction of the identity key first and then the Den, and a minimal
  Tombstone; `werewolfd` exits 67 and is not restarted.
- **Automatic governance sync** (Stage31): pull only, on active admitted
  nodes, with a local off switch.
- **Robustness** (Stage22R, Stage32): real-WAN restart, crash, path loss,
  target failure, resource and storage fault records; a sandbox campaign
  under netem impairment, wall-clock jumps and stalls, which found and fixed
  three initiator-side defects (F1 encrypted-TCP setup retry, F4
  `TCP_NODELAY`, F5 QUIC handshake confirmation) and documented the limits.

Every addition is additive to the rc.1 CLI, wire and persistence
contracts, consolidated in the `docs/V1_*` contracts of the source tree.

The archive targets Linux x86_64 / x86_64-unknown-linux-gnu, tested on
Debian 13.6 and glibc 2.41. It contains werewolfd, werewolfctl, the systemd
unit, install/uninstall scripts, operator guides, checksums, dependency
inventory, third-party notices and the security remediation records.

## Security remediation before the candidate

Four rustls advisories against 0.23.45 (GHSA-c8m5-c952-6cm4,
GHSA-h2v8-394m-p7vv, GHSA-w44r-93j7-qv28, GHSA-vp88-798v-h67f) were
remediated by moving to rustls 0.23.46, with no Werewolf code change. See
the [Stage33 rustls remediation](docs/STAGE33_RUSTLS_0_23_46_REMEDIATION.md).

The candidate's advisory gate then reported five quinn-proto advisories
(RUSTSEC-2026-0338 to -0342), rustls-webpki RUSTSEC-2026-0382 and, in the
lockfile only, webpki-root-certs RUSTSEC-2026-0383. They were remediated
by moving to quinn 0.11.12 / quinn-proto 0.11.18, rustls-webpki 0.103.16
and webpki-root-certs 1.0.10, again with no Werewolf code change. tokio
stays at 1.52.3: its unsound-warning RUSTSEC-2026-0386 (`AsyncFd`) is
accepted, since nothing shipped uses `AsyncFd`, and tokio 1.53.3 lengthened
routed encrypted-TCP tails under loss. See the
[Stage33 QUIC and webpki remediation](docs/STAGE33_QUIC_TLS_DEPS_REMEDIATION.md).
rc.1's [Stage20S remediation](docs/STAGE20S_RUSTLS_2026_0285_REMEDIATION.md)
is kept. The candidate's advisory audit result is in its Stage33 audit
record.

## Upgrade and compatibility

An rc.1 Den starts under rc.2 unchanged. Back up the complete Den before
the upgrade. Applying a first governance genesis moves the protected
manifest to schema 2, which rc.1 binaries refuse, and cannot be undone: a
governed node rolls back only by restoring a backup taken before its
genesis. See INSTALL.md.

## Security boundaries

Public-key trust is configured out of band. Pack membership, a service
name, a discovered record, a route or a governance statement never grants
target access by itself. A relay learns who talks to whom, not the target
or payload. Seven governance keys are the network's authority by
definition; OS root still controls its own host's node. Destruction makes
no claim about storage media. Traffic metadata, denial of service, other
platforms and the documented robustness limits are in
[Known limitations](KNOWN_LIMITATIONS.md).
