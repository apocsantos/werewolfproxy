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
  between a relay and the destination over a real WAN (sandbox only, Stage39);
  destruction under a full disk.

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

Found by Stage39 ([M1](docs/STAGE39_M1_LIVENESS_REPRODUCTION.md)):

- in 1.0.0-rc.2, the receiver's connection to its target has no keep-alive
  or `TCP_USER_TIMEOUT`. On a relay that connection is the hop to the
  destination, so if that path fails silently, or with routing errors, the
  relay keeps the routed session, its target socket and its authority lease
  (part of the requester's per-peer quota) long after the destination is
  gone: beyond 390 s in every sandbox run, and in idle sessions with no
  error at the requester. The kernel's retransmission limit eventually ends
  it. Fixed on `main` after rc.2
  ([M2](docs/STAGE39_M2_TARGET_LIVENESS_FIX.md), `3f1477a`): the target
  connection now has the session sockets' liveness bounds, and the sandbox
  shows release in 56–81 s with a reset at the requester
  ([M3](docs/STAGE39_M3_REGRESSION.md)). The fix is not in the rc.2 archive
  and has not been run over a real WAN.

Found by Stage41 ([M1](docs/STAGE41_M1_INITIATOR_EXPOSURE.md)):

- in 1.0.0-rc.2, the QUIC initiator (every outbound QUIC connection:
  direct and failover Fangs, the routed hop to a relay, discovery) runs
  Quinn's default transport limits. An authenticated receiver can open up
  to 200 streams toward it that it never reads, and send it datagrams:
  249 MB was accepted unread on one connection in 8 s in a unit
  measurement. Only a Pack peer the initiator chose and pinned can do
  this. Fixed on `main` after rc.2
  ([M2](docs/STAGE41_M2_INITIATOR_BOUNDS_FIX.md), `5449261`): the
  receiver gets no stream credit and no datagram buffer, and the
  connection receive window is 1.25 MB (1.29 MB accepted in the same
  measurement). The data-path windows are unchanged, and the Stage32
  QUIC cells keep their verdicts ([M3](docs/STAGE41_M3_REGRESSION.md)).
  The fix is not in the rc.2 archive. The initiator's stream receive
  window (1.25 MB) and send window (10 MB) remain above the receiver's
  bounds, by design: they are local throughput choices, and the send
  window is the Stage34 surface. The number of Fang connections, and so
  the initiator's total memory, was not bounded in rc.2; Stage42 bounds it
  on `main` (below).

Found by Stage42 ([M1](docs/STAGE42_M1_FANG_FLOOD_REPRODUCTION.md)):

- in 1.0.0-rc.2, a Fang accepts every local connection without limit. Any
  host that can reach a Fang's bind address can open connections: local
  processes for a loopback bind, more for any other address. A flood takes
  the peer's whole 64-session quota at the receiver, so the operator's own
  sessions through that peer fail. A plain-TCP Fang has no remote quota
  and grew to 1017 descriptors under 500 connections. On descriptor
  exhaustion (`EMFILE`) the encrypted-TCP, plain-TCP, routed and failover
  Fangs stop listening for good while the daemon keeps running, and the
  QUIC Fang retries `accept` in a tight loop, holding a core and logging
  every failure (in M1 it filled a 3.9 GB `/tmp` in about 10 minutes).
  Fixed on `main` after rc.2
  ([M2](docs/STAGE42_M2_FANG_CEILING_FIX.md), `e9fb6e2`): every Fang kind
  allows 64 concurrent local connections, with 1024 across all Fangs; a
  connection over either ceiling is accepted and reset at once; an accept
  error backs off 100 ms and keeps listening; both are logged at most once
  per 10 s per Fang. Fangs survive descriptor exhaustion and recover, and
  a plain-TCP flood is held at 64 sessions and 145 descriptors
  ([M3](docs/STAGE42_M3_REGRESSION.md)). The fix is not in the rc.2
  archive. **What remains:** a flood on one Fang still denies that Fang's
  other users while it lasts (its 64 slots are full, so new connections
  are reset), and Fangs to the same peer still share that peer's
  64-session quota. There is no per-client fairness within a Fang. A Fang
  bound to a non-loopback address accepts from that whole network; this is
  documented behaviour and was not changed (S42-D5). Stage43 (below)
  addresses part of this on `main`.

Found by Stage43 ([M1](docs/STAGE43_M1_FAIRNESS_REPRODUCTION.md)):

- in 1.0.0-rc.2 and on `main` after Stage42, a Fang has no fairness
  among its clients. Connections that sit idle keep their slots
  indefinitely, so a local client that leaks 64 idle connections locks
  every other client out of that Fang until it exits. On a Fang bound to
  a non-loopback address, one source address can take all 64 slots and
  lock out every other host. Fixed on `main` after rc.2
  ([M2](docs/STAGE43_M2_FAIRNESS_FIX.md), `faa1da2`): when a Fang is
  full, its longest-idle connection is ended (RST to its client) to
  admit a newcomer, if no byte has passed either way for 60 s; and a
  non-loopback source address holds at most 16 of a Fang's 64 slots
  (loopback is exempt). Activity is recorded in the shared copy paths;
  the Stage32 cells that exercise them keep their verdicts, and both
  scenarios stay fixed in three rounds for QUIC, encrypted-TCP and
  plain-TCP Fangs ([M3](docs/STAGE43_M3_REGRESSION.md)). A Fang bound to
  a non-loopback address now logs one startup line saying it is
  reachable from that network; the bind behaviour is unchanged. The fix
  is not in the rc.2 archive. **What remains:**
  - an **active** flood from loopback (the operator's own user, or
    another local user on a multi-user host) still fills a Fang and
    resets its other clients while it lasts: loopback clients share one
    address, and active connections are never evicted;
  - while a Fang is full, a legitimate session idle for 60 s or more (an
    idle SSH session, say) can be ended to admit a newcomer. A Fang
    under its ceiling never evicts;
  - Fangs to the same peer still share that peer's 64-session quota, first
    come, first served (S43-D4, recorded only);
  - when the target closes first, QUIC and plain-TCP Fangs keep the
    half-closed session, and its slot, until the client closes (existing
    TCP half-close behaviour; such a session counts as idle and can be
    evicted);
  - not done: a per-local-user share (F2), an operator-set ceiling per
    Fang (F4) and a per-Fang share of a peer's quota (F5).

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
  path loss between the relay and the destination was exercised only in the
  sandbox (Stage39, below), not over a real WAN.

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
