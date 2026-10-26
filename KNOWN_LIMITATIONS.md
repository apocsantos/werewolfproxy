# Known limitations — 1.0.0-rc.1

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

## On `main` after 1.0.0-rc.1

The list above describes the frozen 1.0.0-rc.1 archive and stays true for
it. Later work on `main` is not in that archive; it changes the picture as
follows.

- **Since exercised (on later builds, not on the rc.1 archive):** the
  Stage22R real-WAN records cover receiver and initiator restarts and hard
  crashes, including systemd's on-failure restart (D01–D04), hard path loss
  and temporary or repeated blackholes (D05A, D05B, D06), target failure
  (D07), resource pressure at production ceilings (D08) and storage faults:
  full, unwritable and read-only storage and torn state (D09). See
  [docs/STATUS.md](docs/STATUS.md).
- **Still not run:** netem-style random loss, jitter and reordering; host
  clock changes; an indeterminate protected-state commit on real storage
  (unit-tested only, D09).
- **Discovery (Stage24, Stage25)** is Pack-only: it finds addresses and
  services of nodes already trusted, never adds trust, and a discovered
  service is usable only where the provider's Target Policy grants it.
  There is no automatic provider selection, no record relaying through
  third nodes and no Kademlia `STORE`.
- **Routing (Stage26)** supports exactly one relay per destination, set by
  the operator (`route set`); there is no route discovery or automatic
  selection; discovery reaches routed Pack members through their relay, but
  nodes learned from contact records are dialled directly. The relay learns
  the requester's identity, the destination's listener address and the
  session's timing and volume, but not the service, the target or the
  payload. Repeated blackholes and hard path loss between the requester and
  the relay were exercised over the real WAN on both hops (Stage26 record);
  path loss between the relay and the destination was not.
- **Governance (Stage28)** covers genesis, key rotation and network
  revocation only. Applying a genesis to a node is irreversible: nothing
  removes governance from a node, a node joins exactly one governance
  network, and a network-revoked key can never be re-added there. Statements
  are delivered by an operator (no gossip), so a node that is not given a
  revocation keeps trusting the member. A fork fences governance with no
  tooling to resolve it. Governance requires a manifest-enabled Den; OS root
  on a host still controls that host's node. Stage29 adds admission and
  activation: after activation the roster bounds every node's Pack, which
  stops one compromised host from widening trust on any other host, but
  not root on that host from rewriting its own Den. `governance sync`
  fetches statements from Pack peers on operator command only; nothing
  propagates automatically, and an unadmitted node still receives its
  admission as a file. Death and the Tombstone are not implemented.

