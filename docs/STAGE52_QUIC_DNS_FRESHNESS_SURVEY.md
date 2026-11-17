# Stage52 — QUIC hostname freshness: reproduction and design

**Delivery update, 2026-11-17:** M2 and M3 are owner-merged in PRs
#163 (`c9e90c8`) and #164 (`3aeb832`). The
[closure record](STAGE52_CLOSURE.md) describes the implemented behaviour
and remaining limits. The reproduction and proposed-contract sections
below retain their original pre-implementation scope.

**Implementation note:** the owner merged this design as PR #162
(`19ea5c2`) on 2026-11-16 and then instructed implementation. The
[M2 record](STAGE52_M2_HOSTNAME_REFRESH.md) tracks that candidate. The
baseline findings and proposed-contract wording below are the historical
pre-implementation record; they are not a claim that the runtime remains
unchanged on a later candidate.

**Status: reproduction and design for review, 2026-11-16. No runtime
change.** The owner authorised the proposed first step: reproduce stale
addresses and design bounded refresh before changing runtime behaviour.
Stage46 deliberately chose resolution at Fang open (S46-D1, A1). Any
per-connection refresh therefore changes that accepted behaviour and
needs the decisions below before implementation.

## 1. Baseline and confirmed code paths

Baseline `main`: Stage51 merge `b9b79d2d39edce207efecae3ee08dd9d67298580`.
Tokio 1.52.3, Quinn 0.11.12, quinn-proto 0.11.19, quinn-udp 0.5.16.
No application source, dependency, trust, wire or transport bound changed
for this reproduction.

| Path | Current resolution | Evidence |
|---|---|---|
| Direct QUIC Fang | Name resolved before open, vector retained for the activation | `main.rs::resolve_fang_names`, `quic_fang.rs::QuicPeerInner` |
| Routed Fang, QUIC relay hop | Relay addresses retained, one reusable peer per activation | `transport/relay.rs::Routed::relay_quic`, `run_routed_forwarder` |
| Failover, direct QUIC provider | Provider addresses retained; endpoints prepared at open | `transport/failover.rs::Path::Quic`, `Ready` |
| Failover, routed QUIC provider | Same retained relay peer | `run_failover_forwarder`, `relay_peer` |
| TCP peer/relay/provider | Name kept and resolved for each dial | `transport/dial.rs::connect_tcp`, `tcp_encrypted.rs` |
| Discovery/sync QUIC queries | Already resolve per query, then use an explicit address snapshot | `discovery/operator.rs`, `discovery.rs::rpc_quic` |

`QuicPeer::dial` races its stored addresses within the 5 s
`hs::READ_WINDOW`. A direct QUIC connection then has a separate 15 s
`hs::CLIENT_WINDOW` for the existing OPEN/ACK work. Failover has its
existing 15 s selection deadline. These are distinct scopes; the whole
session setup is not currently promised to take at most 5 s.

A hostname determines locations, not authority. Every address attempt
uses the captured Pack identity; a routed session also pins the final
destination separately. Routed destinations remain literal addresses.

## 2. Disposable reproduction

Tool: [tools/stage52](../tools/stage52/README.md), reusing the Stage46
namespace and Stage45/46 checkout-only lab. B listens on both loopback
aliases with one process and one generated identity. Private
`moving.stage52` changes from `127.0.0.52` to `127.0.0.53`; resolver
answers are checked explicitly. An established session echoes after the
mapping changes. The old alias is then made unreachable by a private
route, and the new receiver listener remains reachable. The receiver
never restarts. Finally the Fang is closed and reopened.

Three clients per batch, three fresh labs per path were run for direct,
routed and failover paths over both transports. Distinct counted targets
identify the actual failover provider; an echo alone does not establish
that the primary recovered. Expected baseline findings are QUIC resets
or unnecessary failover after the old address is withdrawn, TCP success
at the new address, and QUIC recovery after reopen.

**18/18 scenario runs matched the expected baseline**, campaign exit 0.
All 54 clients before the mapping change and all 54 after the mapping
change (before withdrawal) echoed. All 18 held-session controls echoed
across the mapping change. With the old address withdrawn:

| Path | Nine new clients, after withdrawal | Seconds per client | After Fang reopen (nine clients) |
|---|---|---|---|
| Direct QUIC | 9 resets, no target reached | 5.00 | 9 echoes from B, 0.02 s |
| Direct TCP | 9 echoes from B | 0.01–0.03 | 9 echoes from B, 0.01–0.02 s |
| Routed QUIC hop | 9 resets, no target reached | 5.00 | 9 echoes from C through B, 0.03–0.04 s |
| Routed TCP hop | 9 echoes from C through B | 0.08–0.14 | 9 echoes, 0.03–0.10 s |
| Failover QUIC | 9 echoes from fallback C; reachable primary B unused | 5.04–5.06 | 9 echoes from primary B, 0.02–0.03 s |
| Failover TCP | 9 echoes from primary B; C unused | 0.01–0.04 | 9 echoes from primary B, 0.01–0.02 s |

All daemons survived and all 18 cleanups passed. Resolver answers changed
to the new alias in every scenario; a socket control confirmed that only
the old alias was unreachable. B's process/identity remained the same.
This confirms persistent stale addresses on all three measured QUIC
paths, including unnecessary provider failover despite a reachable primary.

Evidence: `target/stage52-survey/baseline.json`, with an initial single
direct-QUIC smoke case in `smoke.json`. Campaign tool SHA-256:
`9c66404ac9332b4bf6abc76e30c39fff9f6bcd85a177d1ef2b5d4d336c220358`.
All scenarios used daemon SHA-256
`8f1bdb20d54f3babeee487427ccb9a29d4fc4c5d7d08d5b61288f732833995a9`
and lockfile SHA-256
`411ef3b496dfd8c21f97eb2285d55bd0d12ae604436acfdfa16044e80a0d91d0`.
Private generated identities were removed, with no `--keep-temp` use.

This changes the system resolver's private hosts-file input, not a DNS
server. It does not test TTLs, external caches, real dual-stack networks,
WAN migration or an existing session after its network path is removed.
The routed-failover combination is confirmed from code, not measured by
this six-path matrix. No exploit, installed node or real peer is used.

## 3. Options and recommendation

| Option | Behaviour and trade |
|---|---|
| R0 — keep open-time snapshot | Existing behaviour; reopening is required after an address change |
| R1 — resolve for each new hostname connection | Recommended. New independent attempts use the current system resolver result; literals take the existing fast path. Adds resolver work and makes resolver failure affect new sessions |
| R2 — timed cache | Requires a refresh interval and an accepted stale interval; libc lookup results provide no DNS TTL here |
| R3 — refresh after connection failure | Healthy old addresses remain preferred after a DNS change; stale attempts can consume the whole existing 5 s dial budget before refresh |

Per-connection system lookups may generate more observable DNS queries
and add latency to new sessions. A DNS outage can fail new sessions that
the current retained address would still serve. Neither the hosts-file
fixture nor successful reopen measures those operational trades.

Recommend **R1 with open-time validation retained**, per-peer coalescing
of lookups already in flight, and a bound on refresh jobs. There is no
TTL cache or periodic background refresh. A completed answer is not
reused by later independent connection attempts. Concurrent waiters may
share the same in-flight lookup; system/NSS caches still govern what the
resolver returns. This does not promise instantaneous authoritative DNS
freshness.

## 4. Proposed runtime contract (not implemented)

### Name and identity lifetime

- Retain the original peer/relay/provider `HOST:PORT` in long-lived Fang
  configuration alongside its existing captured Pack identity. Keep
  resolving at open, outside the daemon state lock, so bad names still
  fail open with their current error codes and endpoint bind readiness
  remains checked. Open-time answers become readiness/log evidence,
  not a permanent dial list for hostnames.
- At each new hostname dial, obtain a current address vector, interleave
  it and apply the existing 250 ms race/refusal probe. Keep a fixed
  vector for that attempt. Do not restart a race on another DNS answer.
- Preserve explicit address snapshots for short-lived discovery/sync
  queries, which already resolve per query, and preserve literal paths.
  Avoid a second lookup for those operations.
- Keep reusable endpoints at at most one per family per peer. A newly
  returned family creates its endpoint lazily under the existing rule.
  Do not rebuild endpoints or close established connections on refresh.
- Never await resolution while holding the daemon state mutex or an
  endpoint's synchronous mutex. The name never replaces key pinning,
  target authorization, route authorization or ACK validation.

### Time and failure semantics

- Start the existing 5 s dial deadline **before** waiting for a resolver
  slot or lookup. Slot wait, lookup and address race share that deadline;
  they must not become 5 s for resolution plus a fresh 5 s for dialing.
  The existing post-dial handshake and outer failover deadlines remain.
- A name that fails to resolve after open fails this new attempt as
  `dial`, not `handshake`. Do not silently fall back to an earlier answer.
  A subsequent connection can resolve again; failover can try its next
  provider within the unchanged selection deadline.
- An address with the wrong key fails the authenticated handshake as
  today. No application bytes reach it and no unauthenticated fallback
  transport is introduced. An existing established session keeps its
  connection and is not moved or replayed.
- Keep provider order and the commitment boundary: only failures before
  the verified ACK permit failover. DNS changes after commitment do not
  retry application data or switch providers.

### Resolver work must remain bounded

The installed Tokio 1.52.3 source (`src/net/addr.rs`) runs hostname
resolution using `spawn_blocking` and `std::net::ToSocketAddrs`.
Its `src/task/blocking.rs` documents that started blocking jobs cannot
be aborted. Dropping a 5 s lookup future therefore does not establish
that the underlying libc work stopped. Adding only a timeout around
per-session `lookup_host` would leave this resource question unresolved.

Proposed bound for the new refresh path: **16 outstanding resolver jobs
per daemon**, plus at most one in-flight lookup per reusable QUIC peer.
This is a proposed new bound, not an existing certified value. Acquire
capacity asynchronously, inside the caller's dial deadline, before
submitting blocking work. The capacity permit must remain owned by the
blocking operation until it actually completes, including after all
waiters time out or the Fang closes. Keep the in-flight identity until
completion too; a new waiter must not spawn a duplicate hung lookup.
Completed results may serve current waiters, then are discarded.

Implement this through explicit owned blocking work/coordinator state;
do not release a semaphore merely because a timeout dropped
`lookup_host`. Queueing waiters must also stay within the existing Fang
connection ceilings and dial deadline. No jobs start without capacity.
Tests must demonstrate permit lifetime, waiter cancellation and Fang
closure with an intentionally blocked fake resolver before M2 is accepted.

This cap covers the newly introduced ongoing QUIC refresh work, not all
existing resolver use by TCP, open-time validation or discovery. Saturated
refresh capacity can make unrelated hostname QUIC dials time out; literal
dials remain unaffected. A libc call can outlive its requester and delay
runtime shutdown. A hard cancellation or shutdown guarantee would need
a different resolver boundary; it is not claimed by this proposal.

## 5. Proposed implementation and regression steps

- **M1 (this step):** preserve baseline outcomes and provenance, publish
  this design for owner review. No runtime patch.
- **M2 (after decisions):** introduce the hostname source and bounded
  resolution mechanism; use it for direct QUIC, QUIC relays and QUIC
  providers, including routed failover. Unit tests must cover changing
  answers, failure/recovery, no stale fallback, unchanged literals,
  wrong-key rejection, IPv4/IPv6 changes, a shared 5 s deadline, in-flight
  coalescing and cancellation-safe job capacity. Preserve open-time
  error semantics and all existing resource/authentication limits.
- **M3:** reuse this migration matrix with success expected without
  reopen; add NXDOMAIN/recovery, slow/blocked resolution, wrong-key and
  routed-failover fixtures. Check provider order, no retry after ACK,
  established sessions across a mapping change and unchanged literals.
  Repeat Stage46 address controls, Stage49 refusal/silent controls,
  Stage42/47 descriptor/CPU pressure, Stage12 proofs and required
  Stage32 S2/S3/S4/S7 P0 cells. Run Rust checks and full local CI.
- **M4:** update current limitations and closure only from the reviewed
  passing runtime candidate. Do not label this reproduction as a fix.

Frozen rc.1/rc.2 artifacts stay unchanged. S3-P2/S2-P4 remain EXCLUDED,
Stage38 remains CLOSED, and Tokio stays 1.52.3. A new release, WAN work
and hosted-CI/billing changes are outside scope.

### Validation of this reproduction/design change

Full disposable local CI: **13/13 PASS**, actual exit 0, at
`target/ci-local/20261116T182441Z/summary.tsv`, including all 68 Python
unit tests and the advisory audit. No tree edits occurred during that
suite; this results note was added after it finished. The new tool's
Python syntax, CLI help, document links and whitespace checks pass.
The exact tool/lock hashes were checked against the campaign record.
Application, existing tests, local-CI definitions, lockfile, hosted
workflows and release inputs are unchanged from `b9b79d2`.

## 6. Owner decisions before implementation

| ID | Decision | Recommended |
|---|---|---|
| S52-D0 | Replace Stage46 A1 for ongoing QUIC hostname connections? | R1; retain open-time validation |
| S52-D1 | Scope | Direct, routed and failover QUIC, including routed providers; existing per-query resolution and literal destinations unchanged |
| S52-D2 | Resolver failure after open | Fail the new attempt, no stale-answer fallback; retain existing pre-ACK failover |
| S52-D3 | Time budget | Resolution/capacity wait and race share the existing 5 s dial deadline; other deadlines unchanged |
| S52-D4 | New refresh work bound | 16 outstanding jobs per daemon, one in flight per reusable QUIC peer; capacity retained until blocking work completes |
| S52-D5 | Runtime authorisation | Approve M2–M4 only after reviewing reproduction, tradeoffs and cancellation/resource tests above |
