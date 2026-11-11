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
- **Stage44, sandbox** (unprivileged network namespace, unique-local IPv6
  addresses on loopback): QUIC, encrypted-TCP and plain-TCP Fangs, routed
  Fangs over QUIC and TCP hops, discovery and governance sync over both
  transports, a dual-stack receiver, and the per-source share, all over
  IPv6. See [docs/STAGE44_M3_REGRESSION.md](docs/STAGE44_M3_REGRESSION.md).
- **Stage46, sandbox** (unprivileged network namespace with a private
  resolver, and an interface that silently drops one range): a peer, relay,
  provider or target hostname whose first address is refused, silent, or
  `::1` against an IPv4-only listener, on every Fang kind, routed hops,
  discovery and governance sync. See
  [docs/STAGE46_M3_REGRESSION.md](docs/STAGE46_M3_REGRESSION.md).
- **Not run:** the Stage32 impairments over a real WAN; IPv6 on a real
  network (router advertisements, path MTU, a WAN) or with the Stage32
  impairments; real host
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

Found by Stage44 ([M1](docs/STAGE44_M1_IPV6_REPRODUCTION.md)):

- in 1.0.0-rc.2 and on `main` before Stage44, QUIC to an IPv6 peer fails
  everywhere, before a packet is sent: the QUIC client endpoint is bound
  to `0.0.0.0:0`, and quinn refuses an IPv6 destination on it. That breaks:
  - QUIC Fangs and the QUIC legs of routed and failover Fangs, which reset
    every session. The Fang still shows `active` and nothing is logged;
  - discovery and governance sync over QUIC, which report the peer as
    `rejected`.

  QUIC is the default transport, so an IPv6-only peer was reachable only
  over TCP. Also, the Stage43 per-source share counted each IPv6 address
  separately. One host holding a /64 could therefore take all 64 slots of
  a Fang bound to an IPv6 address.

  Fixed on `main` after rc.2 ([M2](docs/STAGE44_M2_IPV6_FIX.md),
  `716ec8a`):
  - a QUIC client endpoint dialling an IPv6 destination binds `[::]:0`.
    Every other destination keeps the IPv4 endpoint unchanged, so a host
    with IPv6 disabled is unaffected;
  - IPv6 sources share by /64;
  - `werewolfctl service connect --bind ::1` is recorded as `[::1]:PORT`.

  Every IPv6 scenario passes in three rounds, and the IPv4 results hold
  ([M3](docs/STAGE44_M3_REGRESSION.md)). The fix is not in the rc.2
  archive.

  **What remains:**
  - IPv6 was run only in a namespace sandbox on unique-local loopback
    addresses, never on a real IPv6 network or a WAN;
  - a peer address that is a hostname is dialled at its first resolved
    address only, with no fallback between addresses or families. Stage46
    (below) fixes this on `main`;
  - the /64 share is a heuristic. A host holding a larger prefix (a /56 or
    /48) can still take more than 16 slots, and hosts sharing one /64
    share 16;
  - a QUIC dial failure is still not logged, and discovery and sync still
    report a local connection error as `rejected`, the status of a trust
    refusal. Stage45 (below) fixes both on `main`.

Found by Stage45 ([M1](docs/STAGE45_M1_VISIBILITY_REPRODUCTION.md)):

- in 1.0.0-rc.2 and on `main` before Stage45, a Fang whose sessions fail
  to set up looks healthy. That covers a peer that is down, that does not
  grant the target, or that the Fang cannot reach:
  - QUIC, encrypted-TCP, routed and failover Fangs log nothing; the
    client sees only a reset;
  - `fang active` still shows the Fang `Active`, with no failure field;
  - a plain-TCP Fang whose target is down logs one line per connection
    (500 connections: 501 lines, 52.6 kB, in 0.1 s);
  - discovery and sync report a local QUIC dial failure as `rejected`,
    the status of a trust refusal;
  - a QUIC Fang to a peer whose Pack address is a hostname opens, then
    resets every session and logs `bad server addr` per connection, with
    no Fang id.

  Fixed on `main` after rc.2 ([M2](docs/STAGE45_M2_VISIBILITY_FIX.md),
  `35f0e3f`):
  - every Fang kind logs a failed session setup with its Fang id, peer and
    class (`dial`, `handshake` or `target`): the first failure, then at
    most one line per 10 s per Fang and class, with the number skipped;
  - the plain-TCP lines go through the same limiter, so the 500-connection
    flood logs one line;
  - `fang active` shows `sessions_failed` and `last_failure` (class and
    age);
  - discovery and sync report a QUIC dial that is refused locally or not
    answered as `unreachable`. Every refusal by a reached node still
    reports `rejected`;
  - a QUIC Fang resolves a hostname peer once, when it opens, and fails
    the open with `QUIC_FANG_FAILED` if the name does not resolve.

  The results hold in three rounds; the Stage42 flood scenarios and the
  Stage32 S2, S3, S4 and S7 P0 cells pass
  ([M3](docs/STAGE45_M3_REGRESSION.md)). The fix is not in the rc.2
  archive.

  **What remains:**
  - a hostname is still dialled at its first resolved address only (QUIC
    Fangs, discovery and sync). Where `localhost` resolves to `::1` first
    and the peer listens on `127.0.0.1` only, as on the test host, a QUIC
    Fang to `localhost` fails every session. The failure is now logged as
    `dial` and counted; a TCP Fang to the same address works. Stage46
    (below) fixes this on `main`;
  - a setup failure caused by a local resource limit (`EMFILE`, `ENFILE`,
    `ENOBUFS`, `ENOMEM`) is labelled `handshake`, not `dial`, and
    discovery reports it `rejected`. Only the label is wrong; the count,
    the rate limit and the session outcome are correct. Stage46 (below)
    fixes this on `main`;
  - the initiator cannot tell why a receiver refused a session (no grant,
    Silver, key mismatch): all are `handshake`. This is by design; the
    receiver does not disclose its reasons;
  - each client of a QUIC Fang whose peer does not answer still waits the
    5 s read window before its reset.

Found by Stage46 ([M1](docs/STAGE46_M1_DIAL_REPRODUCTION.md)):

- in 1.0.0-rc.2 and on `main` before Stage46, a Pack address that is a
  hostname with several addresses is dialled four different ways:
  - a QUIC Fang, discovery and sync over QUIC dial the first address only.
    A first address that is refused, silent or of the other family fails
    every session or query after 5 s, though the next address works;
    `localhost` (`::1` first) is the everyday case;
  - a routed Fang whose QUIC relay is a hostname opens, then resets every
    session at once, labelled `handshake`;
  - a failover Fang whose QUIC provider is a hostname is refused at open;
  - TCP tries the addresses in turn, but a silent first address uses the
    whole window, so the next is never tried: 5 s per session, 10 s on an
    encrypted-TCP Fang (the Stage32 F1 retry repeats it).

  Also, a local resource error on the dial was labelled `handshake` and
  reported `rejected` (Stage45 M3).

  Fixed on `main` after rc.2 ([M2](docs/STAGE46_M2_DIAL_FIX.md),
  `4dee787`):
  - every dial (every Fang kind, routed hops, failover providers,
    discovery, sync, a plain-TCP target) tries every address the name has:
    the first at once, the next 250 ms later or as soon as the previous
    fails, alternating IPv6 and IPv4, the first to connect winning, all
    within the dial's existing 5 s window. A literal address is one
    attempt, as before;
  - QUIC resolves a name once, when the Fang opens, keeping every address,
    for the peer, a QUIC relay and each failover provider; a name that does
    not resolve fails the open with the reason. A QUIC socket for the other
    address family is opened only when one of its addresses is tried;
  - a local resource error on the dial is `dial`, and `unreachable` in
    discovery and sync.

  Every path reaches a name's live second address in about 0.3 s, in three
  rounds; the Stage45, Stage44 and Stage42 scenarios and the Stage32 S2
  P0/P1, S3, S4 and S7 P0 cells pass ([M3](docs/STAGE46_M3_REGRESSION.md)).
  The fix is not in the rc.2 archive.

  Found and fixed by Stage47 ([survey](docs/STAGE47_FLOOD_CPU_SURVEY.md),
  [M1](docs/STAGE47_M1_CPU_REPRODUCTION.md)): with the Stage46 dialer, an
  encrypted-TCP Fang's daemon out of file descriptors used about 12% more
  CPU per episode (1.82 s against 1.63 s; none on a QUIC Fang, none in
  steady state). Stage46 M3 had stated 0.8–8.8% against 1.2–1.8%, from a
  5 s sample that depends on when the work ends. A single address is now
  dialled without the race's task machinery
  ([M2](docs/STAGE47_M2_CPU_FIX.md), `e3d89f9`): 1.62–1.74 s per episode
  in three more rounds, the pre-Stage46 level, with the Stage46 matrix,
  the Stage42 floods and the Stage32 S2, S3 and S4 P0 cells unchanged
  ([M3](docs/STAGE47_M3_REGRESSION.md)). Neither the cost nor the fix is
  in the rc.2 archive. `tools/stage42/fang_flood.py` now also reports
  `cpu_seconds_flood` (cumulative over the episode), the figure to
  compare between builds; `cpu_flooded_pct` is kept for older records.

  Found and fixed by Stage48 ([survey](docs/STAGE48_TCP_SETUP_RETRY_SURVEY.md),
  [M1](docs/STAGE48_M1_RETRY_REPRODUCTION.md)): the Stage32 F1 setup
  retry also repeated a TCP connect that had timed out, so in failover a
  silent encrypted-TCP provider held each local connection 10 s before
  the next provider (a QUIC provider: 5 s), and two silent ones used up
  the 15 s selection window: every client was reset and a live third
  provider was never tried. Failover's encrypted-TCP leg now takes a
  connect timeout as final and tries the next provider at once; every
  other F1 retry is unchanged ([M2](docs/STAGE48_M2_RETRY_FIX.md),
  `3c7bbff`). One silent TCP provider now costs about 5 s and two about
  10 s, as QUIC providers do, in three rounds, with the Stage46 matrix
  and the Stage32 S2, S4 and S7 P0 cells unchanged
  ([M3](docs/STAGE48_M3_REGRESSION.md)). The fix is not in the rc.2
  archive.

  **What remains:**
  - why one spawned task and `select!` per dial cost about 0.27 ms under
    descriptor exhaustion was not investigated (Stage47 M2). A hostname
    with several addresses still takes the race and so may still pay it;
    no measured case;
  - a direct encrypted-TCP Fang whose peer does not answer still resets
    each client at 10 s: the Stage32 F1 retry repeats the connect, by
    owner decision (S48-D2; QUIC and plain TCP: 5 s). Removing it there
    would need the F1 loss validation (Stage32 S1-P2) rerun;
  - in failover, a provider whose connect gets no answer for the whole
    5 s window (every SYN or SYN-ACK lost) is passed over, not dialled
    again; the next provider takes the session. F1's validation saw no
    such case in 600 sessions under 10% loss (Stage48 M2 §5);
  - QUIC resolves a name only when the Fang opens: after a DNS change the
    Fang must be reopened. TCP, discovery and sync resolve at each dial;
  - the dialer was run only in the namespace sandbox, on loopback and
    unique-local addresses, never on a real dual-stack network or a WAN;
  - from Stage45: a QUIC peer that does not answer still costs each client
    the 5 s window, and the initiator still cannot tell a receiver's
    refusal reasons apart (by design).

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
