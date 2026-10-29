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
- **Since exercised in a sandbox (Stage32):** netem random and burst
  loss, delay, jitter, reordering, duplication, corruption and rate
  limits, in unprivileged network namespaces on one host, and wall-clock
  jumps (libfaketime, one process) and process stalls (`SIGSTOP` 30 s and
  300 s). See
  [docs/STAGE32_ROBUSTNESS_SPEC.md](docs/STAGE32_ROBUSTNESS_SPEC.md) and
  [docs/STAGE32_TIME_CERTIFICATION.md](docs/STAGE32_TIME_CERTIFICATION.md).
  Documented limits found there:
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
- **Still not run:** the same impairments over a real WAN, IPv6; real
  host (kernel) clock changes and suspend/resume; an indeterminate
  protected-state commit on real storage (unit-tested only, D09).
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
  are delivered by an operator, and after activation also fetched
  automatically from Pack members (Stage31, pull only, no gossip); a node
  that receives neither keeps trusting a revoked member. A fork fences governance with no
  tooling to resolve it. Governance requires a manifest-enabled Den; OS root
  on a host still controls that host's node. Stage29 adds admission and
  activation: after activation the roster bounds every node's Pack, which
  stops one compromised host from widening trust on any other host, but
  not root on that host from rewriting its own Den. `governance sync`
  fetches statements from Pack peers, on operator command and (Stage31)
  automatically on active, admitted nodes; the ceremony stays manual, an
  unadmitted node still receives its admission as a file, and a revoked
  node cannot learn its own revocation by sync (its former peers refuse
  it), so it receives that as a file too.
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

