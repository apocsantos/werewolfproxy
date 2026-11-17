# Stage53 — fairness between Fangs sharing a peer quota

**Implementation note:** the owner merged this design in PR #166
(`4fed878`) and instructed M2 implementation. The
[M2 record](STAGE53_M2_PEER_QUOTA_ADMISSION.md) tracks that candidate.
The baseline and proposed-contract wording below retain their original
pre-implementation scope.

**Status: reproduction and design for review, 2026-11-17. No runtime
change.** The owner authorised the proposed first step after Stage52:
reproduce two-Fang contention, compare admission policies and submit a
design before implementation. This revisits Stage43 scenario E/F5,
previously recorded only under S43-D4, as its own stage.

## 1. Baseline and the resource being shared

Baseline `main`: Stage52 closure merge
`622766ec90f9e742ee26b0e9765e6eda3bedd84d`. Tokio 1.52.3, Quinn 0.11.12,
quinn-proto 0.11.19 and quinn-udp 0.5.16. No application source, dependency,
wire message, resource bound, gate or release input changes in this step.

The [Stage43 closure](STAGE43_CLOSURE.md) and
[current limitations](../KNOWN_LIMITATIONS.md) record the shared quota:

| Layer | Current rule | Consequence |
|---|---|---|
| Local Fang admission, `transport/fang_accept.rs` | 64 connections per Fang; 1,024 per daemon | Two Fangs can each admit local clients, independently |
| Local fairness, same module | Idle eviction after 60 s under Fang pressure; 16 per non-loopback source, IPv6 /64; loopback exempt | Neither rule partitions a remote peer's quota between Fangs |
| Receiver authority, `authority.rs::reserve` | 64 establishing + active sessions per authenticated requester identity; 1,024 globally | All that requester's Fangs compete, even across QUIC and TCP |
| QUIC connection authority | Additional 64-session per-connection ceiling | Does not provide fairness between independent connections/Fangs |

The receiver authenticates the requester, not the originating Fang.
Changing a local bind port, target, transport, hostname or resolved IP
does not create another requester quota. Creating or rotating identities
to evade the receiver's limit is not an admission policy.

## 2. Disposable reproduction

[Tool and procedure](../tools/stage53/README.md): receiver A, initiator B
with two Fangs opened before load, and independent initiator C. All use
fresh generated identities and loopback ports. Separate counted echo
targets identify actual payload delivery. C holds one session throughout.
Test all four ordered transport pairs: QUIC/QUIC, TCP/TCP, QUIC/TCP and
TCP/QUIC. Reverse which Fang fills first without reopening either Fang.

The saturated owner must echo on 64 sessions. Receiver-local admin status
must show 65 active and zero establishing (B64 + C1). The sibling must
reset three clients immediately and three more after five seconds of
repeated payload exchange on held sessions. Three new C clients must
echo during B's saturation, excluding receiver-wide exhaustion. Denied
sibling clients must produce zero target payload streams.

Release exactly one B session: A must report 64 active, then the sibling
must immediately echo and hold the recovered slot, restoring 65. The
original owner must now be reset. Release all B sessions: A returns to
C's one, and both B Fangs must recover without reopening. Reverse roles
and repeat. Expected denial confirms the baseline limitation; it is not
a fix or a new service-availability guarantee.

**12/12 scenarios CONFIRMED**, campaign actual exit 0, three fresh labs
per pair and **24 arrival-order phases**. All daemons survived, all
cleanups passed and all private fixture directories were removed.

| Transport pair (both arrival orders, three rounds) | Sibling reset seconds | Echo after one slot released, seconds |
|---|---|---|
| QUIC / QUIC | 0.0154–0.0458 | 0.0196–0.0250 |
| Encrypted TCP / encrypted TCP | 0.0063–0.0264 | 0.0174–0.0415 |
| QUIC / encrypted TCP | 0.0063–0.0275 | 0.0140–0.0404 |
| Encrypted TCP / QUIC | 0.0062–0.0311 | 0.0160–0.0355 |

Every phase verified 64 owner echoes (**1,536 total**), six sibling resets
(**144 total**) with zero target payload streams, three new independent
peer echoes (**72 total**), and continuing echoes from all 65 held sessions.
After one release all **24** siblings acquired the slot, and the original
owner was then reset in all 24 phases. After full release both Fangs
recovered in all **48** probes without reopening. All recorded authority
snapshots matched the expected 65 → 64 → 65 → 1 active-session sequence,
with no establishing sessions left at the checkpoints. No log guard fired.

Tested fixture commit: `b6b166181a771c4787c1f65385792431e0f37199`.
Evidence: `target/stage53-m1/matrix.json`, `matrix.log`, `verified.json`.
Every scenario used daemon SHA-256
`92ba4d1589a95499fa65bce70195bdfc7165181afb3c9a51e28f912a7b847211`,
the same as Stage52 M3. All **307** recorded M3 source-input hashes still
match; lockfile SHA-256 is
`411ef3b496dfd8c21f97eb2285d55bd0d12ae604436acfdfa16044e80a0d91d0`.
The matrix also records exact tool/helper hashes. This is local functional
evidence, not a comparison of transport latency or throughput.

The first sandboxed smoke failed at daemon startup (exit 65), before any
quota measurement. It is retained in `smoke.json`; the unchanged fixture's
host smoke passed (`smoke-host.json`). No assertion or daemon code was
relaxed. The final fixture added the independent-peer new-admission
control and failure-log retention before the separate final matrix.

## 3. Goal and policy options

The concrete goal is to stop one Fang from taking every session available
to its already-open sibling Fangs on the same initiator. The receiver's
security quota stays 64. This is admission fairness between configured
Fangs, not between local clients/users or independent initiator processes.
No admission policy creates capacity when all permitted sessions are busy.

| Option | Behaviour | Tradeoff |
|---|---|---|
| Q0 — retain first come, first served | Record the limitation; no new local policy | A busy Fang can keep denying its siblings |
| Q1 — operator-set per-Fang caps | Operator partitions capacity by workload | New persistent/control fields; manual coordination; defaults alone do not fix contention |
| Q2 — equal reserved shares among open Fangs | Partition a local 64-session budget for each receiver identity; no borrowing | Predictable shares for already-open Fangs, but unused shares reduce utilisation; changing membership cannot reclaim active sessions without disruption |
| Q3 — lend unused shares, queue fairly | Use spare capacity and order waiting requests between Fangs | An active borrower can hold every slot indefinitely; fair queueing cannot reclaim occupied sessions; queues need explicit bounds and deadlines |
| Q4 — preempt active sessions | Reclaim an existing session when a sibling needs room | Breaks established applications, including legitimate transfers; reconnect churn; requires a new termination policy |
| Q5 — receiver/wire allocation | Receiver negotiates or enforces allocation by an authenticated Fang identifier | Changes the protocol/trust surface and compatibility; much larger than local admission |

Recommend **Q2, with no active-session preemption**, subject to the
decisions below. It addresses the measured case when sibling Fangs are
already registered before load. It deliberately does not promise bounded
admission for a Fang opened after another already holds all 64 sessions.
If that stronger goal is required, the owner must accept preemption or
a session-lifetime policy; it cannot be obtained by queue ordering alone.

## 4. Proposed Q2 contract — not implemented

### Identity and membership

- One daemon-local coordinator, keyed by the captured authenticated
  receiver identity, shared across QUIC/TCP and all forwarders. Use the
  validated full Pack public key as the grouping identity; do not group
  by name, IP, local port, transport or an unvalidated fingerprint string.
  Remote quotas remain authoritative; this is local cooperation only.
- A Fang registers once per distinct receiver it can consume, before
  activation. Direct authenticated Fangs register their peer. A routed
  Fang registers both relay and destination: both receivers account the
  original requester. Deduplicate a repeated identity. Plain TCP has no
  authenticated peer quota and keeps its existing local rules.
- A failover Fang registers the union of its potential provider and
  route receiver identities. This reserves shares even at unused fallback
  providers: an explicit utilisation cost. Do not reserve live session
  permits for every alternative at once.
- Membership changes and captured Pack/route generations must follow the
  existing activation and quiescence rules. Failed activation rolls back
  all registrations. A closed/invalidated Fang releases membership only
  after its owned session tasks end; retained reservations remain counted.
  Remove coordinator entries once both membership and usage are empty;
  repeated open/close must not retain keys or grow bookkeeping indefinitely.

### Shares and admission

- For each receiver, divide a **local budget of 64** among its N open
  registered Fangs. Each gets `floor(64/N)`; distribute the remainder in
  stable activation order, at most one extra slot per Fang. N=1 permits
  64, N=2 permits 32 each, N=3 permits 22/21/21. Unused shares are not lent.
- Proposed membership bound: **64 Fangs per receiver identity**. A 65th
  registration fails open atomically with a distinct local capacity
  error, before activation. Rejecting an existing configuration is a
  compatibility change requiring the owner's explicit acceptance.
- A local connection still acquires its existing Fang/global/source
  admission slot. Before an outbound authenticated attempt, atomically
  acquire one coordinator reservation for every receiver the attempt
  needs. Both total usage and that Fang's share must permit it. No partial
  acquisition, cross-peer hold-and-wait or unbounded queue. A denied
  direct/routed client resets promptly; record `dial` through the existing
  bounded health/log path, identifying local capacity without adding a
  receiver refusal reason to the wire.
- Membership growth recalculates shares. Existing sessions above a new
  share keep running and remain counted; that Fang admits no more until
  below its new share. Total usage stays at most 64. A new Fang may still
  be denied indefinitely while older active sessions occupy that total.
  Membership shrink increases shares after quiescence. Never evict an
  established session merely to meet the new partition.

### Lifetime, routes and failover

- Reservations count establishing and committed sessions, using RAII
  across errors, cancellation and Fang closure. Hold them through the
  lifetime of the local owned attempt/session and its cleanup. No daemon
  state lock is held across a dial or an await; coordinator bookkeeping
  is short and synchronous. Receiver lease release may lag local closure;
  a local free slot is not proof of remote availability.
- A routed attempt acquires the relay and destination identities together.
  Two destinations sharing one relay compete for that relay's budget.
  Local admission must neither bypass target/route authorization nor create
  a trust relationship. No receiver quota is raised.
- Failover acquires only the current provider's direct/routed demand set.
  Local capacity denial before verified ACK may try the next provider in
  existing order within the existing selection deadline. Release failed
  attempt reservations before trying the next. After verified ACK, retain
  the selected set for the session; never retry or replay payload to free
  capacity. Do not read application data before the existing boundary.
- Do not change resolver coordination, dial/handshake/selection deadlines,
  transport windows, idle eviction, source shares or global local limits.
  Admission bookkeeping does not belong in the payload-copy paths.

## 5. Limits, compatibility and alternatives

Q2 sacrifices utilisation when an open Fang is unused. Administratively
opening Fangs changes shares; an unreachable provider still reserves a
share through its registered Fang. Closing unused Fangs returns shares
after quiescence. Local administrative control is not an untrusted client's
fairness boundary, and this does not prevent active denial within one Fang.

Only one daemon's outgoing sessions are coordinated. Another process
using the same requester key, non-Fang RPC activity or remote exhaustion
can still cause receiver refusals. The local 64 budget assumes the current
receiver convention, not negotiated capacity. An older/future receiver
with a different quota cannot be inferred from a handshake refusal.
Routed destinations and shared relays can constrain an otherwise idle
direct peer. No universal end-to-end fairness or timing guarantee follows.

The first reproduction measures direct, literal IPv4 QUIC/TCP Fangs only.
Routed/failover sharing and later-opened-Fang contention are code-derived
design cases, not measured fixes. No authoritative DNS, WAN, comparative
liveness, throughput improvement or new release claim is made. If fixed
operator caps (Q1) better fit deployments than Q2's idle reservation cost,
choose that explicitly before implementation.

## 6. Proposed milestones and owner decisions

- **M1 (this step):** retain baseline outcomes and publish this design.
- **M2 after review:** implement the selected local admission policy across
  direct, routed and failover authenticated paths. Unit tests must cover
  shares/remainders, activation rollback and membership bounds, key-based
  grouping across transports, atomic multi-key reservations, over-share
  grandfathering, cancellation and cleanup, failover release and post-ACK
  commitment. No runtime change is authorised by this reproduction alone.
- **M3:** three-round two-Fang matrix expecting the selected shares;
  mixed transports, three siblings, one-only/unused/new/closed Fang,
  independent peers, routed common-relay and common-destination contention,
  direct/routed providers, local-denial failover and post-ACK reset controls.
  Verify resource cleanup, bounded CPU/logging and unchanged identity/policy.
  Run workspace fmt/Clippy/tests, Stage12 proofs, full local CI, Stage42/43
  pressure/fairness, Stage46/49 address/refusal, Stage52 resolver/migration
  and S2/S3/S4/S7 P0 regression. Record all actual exits and source hashes.
- **M4:** limitations/status and closure only from reviewed passing evidence.

| ID | Decision | Recommended |
|---|---|---|
| S53-D0 | Revisit Stage43 E/F5 as its own implementation stage? | Yes, after this reproduction/design review |
| S53-D1 | Policy Q0–Q5 and fairness goal | Q2 for already-open Fangs; no claim of bounded admission after late activation |
| S53-D2 | Budget, shares, membership limit | Local 64 budget, equal reserved shares, no borrowing; maximum 64 Fangs per receiver |
| S53-D3 | Existing sessions above a recalculated share | Keep them; deny new admissions until below share; no active preemption |
| S53-D4 | Scope and identity | All authenticated Fang paths, grouping full captured receiver key; atomic routed demand sets; plain TCP unchanged |
| S53-D5 | Failover on local capacity denial | Existing provider order and pre-ACK fallback; no post-ACK retry |
| S53-D6 | Authorise M2–M4 after reviewing utilisation and compatibility costs? | Explicit owner approval of the selected contract before implementation |

## 7. Validation of this reproduction/design change

Full disposable local CI: **13/13 PASS**, actual exit 0, at
`target/ci-local/20261117T155748Z/summary.tsv`, including all 68 Python
unit tests and the advisory audit. No tree edits occurred during that
suite; this validation note was completed afterward. Python syntax and
CLI help, all **74** local document links checked and whitespace checks
pass. Matrix input hashes and all 307 M3 source hashes still match.

The committed changes are the new reproduction, its README, this survey
and the index link. Existing runtime/tests/gate definitions, dependencies,
advisory ignores, workflows and release inputs are unchanged from
`622766e`. No new Rust/proof/chaos campaign is claimed for this tool/docs
step; those controls are required for the proposed runtime implementation.

Frozen rc.1/rc.2 artifacts stay unchanged. S3-P2/S2-P4 remain EXCLUDED,
Stage38 remains CLOSED and Tokio stays 1.52.3. New release, hosted billing
changes, WAN work and identity-based quota evasion are outside scope.
