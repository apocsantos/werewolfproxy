# Known limitations — 1.0.0-rc.2 candidate

This list describes the `1.0.0-rc.2` candidate (Stage33). The list for
the frozen 1.0.0-rc.1 archive is kept unchanged under
[1.0.0-rc.1](#100-rc1), and stays true for that archive.

## Platform and scope

- Certification covers Linux x86_64, target x86_64-unknown-linux-gnu, tested
  on Debian 13.6 / glibc 2.41. Windows, macOS, ARM, BSD, containers and other
  libc baselines are not certified. The release binaries dynamically use the
  tested Linux runtime libraries; no compatibility claim is made for older
  glibc systems.
- There is no Windows service support, GUI, mobile client, pairing protocol,
  remote administration or automatic remote trust provisioning. Peer public
  keys are exchanged out of band.
- Not built, and outside the candidate's architecture-complete scope
  (Stage33 decision 1a): discovery or service advertisement beyond the
  Pack, Kademlia `STORE`, record relaying through third nodes, route
  discovery or automatic route selection, more than one relay, and
  provider selection by reputation, latency or proximity.
- This is a private candidate; nothing is published.

## Security nonclaims

- Stage15B detects partial or mixed protected-state restoration. It does not
  detect a complete internally consistent historical Den or VM snapshot
  without an external freshness anchor.
- Stage16 bounds selected remote concurrency and lifetimes; it does not prevent
  distributed denial of service, bandwidth exhaustion, upstream attacks,
  starvation during sustained pre-auth saturation, or host resource failure
  outside configured bounds.
- Endpoints, transport type, packet sizes, timing and traffic volume remain
  visible. There is no cover traffic, constant-rate transmission, full size
  normalization, timing obfuscation, or anonymity: one relay hides the
  requester's address from the destination, not the requester from the
  relay. Keep-alives, discovery and the periodic automatic governance sync
  are additional observable traffic.
- Root, a privileged administrator, kernel compromise, or offline access to
  the host storage can undermine confidentiality or restore older state.
  OS root on a host still controls that host's node, governed or not.
- WerewolfProxy does not make a compromised application or target trustworthy.

## What has been exercised

- **Stage20** (rc.1): bounded adversarial soak campaigns.
- **Stage22R, real WAN** (builds after rc.1): receiver and initiator
  restarts and hard crashes, including systemd's on-failure restart
  (D01–D04), hard path loss and temporary or repeated blackholes (D05A,
  D05B, D06), target failure (D07), resource pressure at production
  ceilings (D08) and storage faults: full, unwritable and read-only storage
  and torn state (D09). See [docs/STATUS.md](docs/STATUS.md).
- **Stage32, sandbox** (unprivileged network namespaces on one host): netem
  random and burst loss, delay, jitter, reordering, duplication,
  corruption and rate limits on both transports, direct and routed;
  wall-clock jumps (libfaketime, one process, ±60 s to ±8 days) and process
  stalls (`SIGSTOP` 30 s and 300 s). See
  [docs/STAGE32_ROBUSTNESS_SPEC.md](docs/STAGE32_ROBUSTNESS_SPEC.md) and
  [docs/STAGE32_TIME_CERTIFICATION.md](docs/STAGE32_TIME_CERTIFICATION.md).
- **Not run:** the Stage32 impairments over a real WAN; IPv6; real host
  (kernel) clock changes and suspend/resume; an indeterminate
  protected-state commit on real storage (unit-tested only, D09); path loss
  between a relay and the destination; destruction under a full disk.

## Transport and robustness

Documented limits found by Stage32:

- an encrypted-TCP session whose data stays unacknowledged for 60 s
  ends (`TCP_USER_TIMEOUT`), and a QUIC session silent for 30 s ends
  (idle timeout); under sustained burst loss (about 19%) transfers do
  not reliably complete on any transport, though no data was corrupted;
  a peer stalled longer than these ends its sessions;
- QUIC reads reordering as loss: under 50 ± 20 ms jitter its throughput
  is about a third of encrypted TCP's; a QUIC stream carries at most
  64 KiB per round trip (about 620 kB/s at 100 ms RTT), the cost of
  its memory bound; a routed QUIC hop under jitter can be slower than
  the campaign bound while still progressing;
- a node whose wall clock is more than 7 days off real time, in either
  direction, cannot publish a contact record its peers accept, until
  its clock is back.

Found by Stage34 ([record](docs/STAGE34_S3_P2_RECORD.md)):

- with quinn-proto 0.11.18 (the security-remediated version rc.2 ships;
  0.11.19 is the same), a routed QUIC hop under 10% random loss on both
  links can fail at setup or crawl for some loss patterns, where
  quinn-proto 0.11.17 and earlier completed in seconds; no corruption or
  security failure. The Stage32 cell S3-P2 is EXCLUDED, not certified,
  until a later quinn-proto release is retested;
- under the same loss, a routed encrypted-TCP hop can stall for up to
  about 100 s after the relay has forwarded everything, while the
  relay's kernel retransmits its last data (kernel TCP backoff, on every
  build).

## Discovery, services and routing

- **Discovery (Stage24, Stage25)** is Pack-only: it finds addresses and
  services of nodes already trusted, never adds trust, and a discovered
  service is usable only where the provider's Target Policy grants it.
  Provider failover (Stage27) follows the operator's order and happens
  only before a session is established; there is no selection by
  reputation, latency or proximity, no record relaying through third
  nodes and no Kademlia `STORE`.
- **Routing (Stage26)** supports exactly one relay per destination, set by
  the operator (`route set`); there is no route discovery or automatic
  selection; discovery reaches routed Pack members through their relay, but
  nodes learned from contact records are dialled directly. The relay learns
  the requester's identity, the destination's listener address and the
  session's timing and volume, but not the service, the target or the
  payload. Repeated blackholes and hard path loss between the requester and
  the relay were exercised over the real WAN on both hops (Stage26 record);
  path loss between the relay and the destination was not.

## Governance

- **Statements** (Stage28–30): genesis, key rotation, network revocation,
  admission, activation and death, each needing 7 of 10 governance keys.
  Seven compromised keys defeat the scheme by definition, including
  revoking every member or killing the network.
- **Irreversible:** applying a genesis to a node cannot be undone; a node
  joins exactly one governance network, and a network-revoked key can
  never be re-added there. The first genesis moves the manifest to schema
  2, which 1.0.0-rc.1 binaries refuse. Governance requires a
  manifest-enabled Den.
- **Delivery** is pull only, no gossip: a genesis is applied only from a
  file; later statements as files or by `governance sync` from Pack peers,
  and automatically (Stage31) on active, admitted nodes. A node that
  receives neither keeps trusting a revoked member. The ceremony stays
  manual, an unadmitted node receives its admission as a file, and a
  revoked node cannot learn its own revocation by sync (its former peers
  refuse it), so it receives that as a file too.
- **Forks** fence governance, with no tooling to resolve them.
- **Activation** (Stage29) makes the roster bound every node's Pack, which
  stops one compromised host from widening trust on any other host, but
  not root on that host from rewriting its own Den.

## Death and retirement

- **Death and retirement (Stage30).** On death or retirement, Werewolf
  commits the decision, locks the node, destroys the node's identity key
  first and then every file in the Den, overwriting each regular file it
  owns once before removing it, and leaves only a minimal Tombstone. It
  makes no claim about storage media (journals, copy-on-write snapshots,
  SSD wear-levelling), logs, swap, backups or copies outside the Den, nor
  that any governance private key was destroyed. Death travels only as a
  file (`governance sync` cannot deliver it). Destruction under a full disk
  has not been exercised, and the media residue measurement is a separate
  root-run script.
- No timer, clock or network observation ever triggers destruction; local
  retirement can be done by anyone with the node's control socket, and
  affects only that node.

<a id="100-rc1"></a>
## 1.0.0-rc.1

- Certification covers Linux x86_64, target x86_64-unknown-linux-gnu, tested
  on Debian 13.6 / glibc 2.41. Windows, macOS, ARM, BSD, containers and other
  libc baselines are not certified.
- Stage15B detects partial or mixed protected-state restoration. It does not
  detect a complete internally consistent historical Den or VM snapshot
  without an external freshness anchor.
- Stage16 bounds selected remote concurrency and lifetimes; it does not prevent
  distributed denial of service, bandwidth exhaustion, upstream attacks,
  starvation during sustained pre-auth saturation, or host resource failure
  outside configured bounds.
- Endpoints, transport type, packet sizes, timing and traffic volume remain
  visible. There is no cover traffic, constant-rate transmission, full size
  normalization, timing obfuscation, or multi-hop anonymity.
- Root, a privileged administrator, kernel compromise, or offline access to
  the host storage can undermine confidentiality or restore older state.
- WerewolfProxy does not make a compromised application or target trustworthy.
- There is no Windows service support, GUI, automatic remote trust
  provisioning, or pairing protocol. Peer public keys are exchanged out of
  band.
- The release binaries dynamically use the tested Linux runtime libraries;
  no compatibility claim is made for older glibc systems.
- Stage20 did not execute netem packet loss/jitter/reordering, direct ENOSPC,
  read-only filesystem, host clock changes, or systemd restart chaos. Related
  focused persistence, lifecycle, outage and monotonic-time tests exist in
  earlier stage suites, but do not substitute for those specific Stage20
  campaigns.
- The three Cargo workspace packages retain internal package version 0.1.0;
  release binaries and archive identify as 1.0.0-rc.1. This distinction is
  documented in the release audit.
