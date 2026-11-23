# Stage55 M2 — bounded global idle reclamation

**Status: Stage55 M2 owner-merged in PR #175; M3 and M4 are also merged.** The owner merged
[PR #175](https://github.com/apocsantos/werewolfproxy/pull/175) as
`16d67a572a471f090f2e95ab0cca78588e92b42b` and instructed continuation.
The [reviewed proposal](STAGE55_GLOBAL_IDLE_ADMISSION_SURVEY.md) decisions
S55-D0–D6 are applied as recommended: G1, same Fang only, all Fang kinds.
This record documents the implementation; the [M3 regression](STAGE55_M3_REGRESSION.md)
is merged and the [M4 closure](STAGE55_CLOSURE.md) records the remaining limits.

Fixed runtime/test/fixture source:
`37218382c1352e9fa717761706996be7297f49f1`.
All **17** campaign workers and the overall manager exited **0**;
separate scenario verification passed. The suite completed at
**2026-11-22 18:32:55 UTC**. Final documentation notes followed full CI.

## Secure reproduction before runtime changes

The new [admission fixture](../tools/stage55/README.md) first ran QUIC and
encrypted TCP against the unchanged merged runtime. Each lab holds **32
secure sessions plus 992 plain filler sessions**, reaching the actual
1024 global cap while the authenticated receiver holds only 32 sessions.
There is no receiver/peer-quota overrun or lowered runtime limit.

Both full 65-second observations reproduce denial below the owner's local
64-slot ceiling. All old sessions survive; the empty Fang also denies,
the full local 64 positive control replaces one idle session, and releasing
a different Fang's slot restores secure admission. Receiver authority
drains to zero, descriptors return to 16, and both private labs clean up.
Actual baseline process exit is 0. Evidence:
`target/stage55-m2-baseline/secure.json` and its log.

## Resulting behaviour and ownership

When ordinary global acquisition fails and no earlier local/source victim
was used, the Fang may cancel one established own session idle for at least
60 seconds. Selection checks current global pressure and rechecks activity
before marking. Longest idle wins; stable identifier breaks ties. Setup,
recent, already-evicting and sibling sessions are excluded. An empty Fang
has no own victim and still denies; there is no cross-Fang stealing.

Secure establishment remains the final destination's verified ACK, never
an outer relay ACK. Plain TCP has a separate marker set only after its
target connects successfully; an absent peer reservation is not enough.
Releasing a secure reservation clears establishment while the local slot
finishes cleanup, preventing selection of a stale committed candidate.

Each registry entry owns a slot-release notification. `FangSlot` removes
the entry, drops its actual local/global permits, then notifies. Marking
or peer-reservation release alone never grants global capacity. The
waiter waits for that victim at most the existing **1 s**, then tries the
global permit once; it never queues on the global semaphore. A competitor
can win the slot and the newcomer then denies without another victim.

The operation gate is shared with Stage54 peer reclamation and acquired
nonblocking. The newcomer carries one victim budget across local, source,
global and peer triggers and all provider attempts. A prior victim prevents
another global choice; a global victim prevents a later peer/provider
choice. Cancellation releases the operation gate without crediting quota.
Registry locks are short and never cross an await; existing registry →
coordinator lock ordering is unchanged.

## Deadline accounting

Only successful global reclamation carries an earlier setup start: the
instant of its initial global-cap denial. That start bounds the **first
5 s dial/target-connect window** and, for provider Fangs, the **15 s whole
selection window**. The existing one-second cleanup wait is inside these
budgets, not followed by a new full window.

The first dial cutoff is stored separately from Stage54's peer wait cutoff,
so peer reservation bookkeeping cannot clear it. It is consumed at the
first actual dial; the direct TCP F1 retry retains its existing later
window. Subsequent provider attempts retain their per-attempt dial windows
within the earlier whole-selection deadline. Receiver, TLS and per-phase
handshake windows are unchanged. Plain TCP uses the same first-dial cutoff
for its target connect. Paths without global reclamation keep their
existing deadlines and local/source behaviour.

No cap, idle threshold, timeout constant, transport window, dependency,
operator setting, wire message or persistent format changes. Receiver
authorization and captured identities/routes remain authoritative.
Post-ACK failure still resets without provider replay.

## Validation and remaining work

The baseline regression
`global_ceiling_reclaims_an_own_committed_idle_session` failed with actual
exit **101** on the unfixed runtime, then passed with its exact original
test bytes retained. Separate tests cover global eligibility, stable ties,
real release, slow cleanup, cancellation/busy gate, pressure disappearing,
competing acquisitions, shared victim budgets and deadline accounting.
Development evidence is under `target/stage55-m2-red/`.

### Fixed-source results

| Control | Result |
|---|---|
| Workspace, release and audits | Format, Clippy with warnings denied, all **513** Rust tests, locked release build and both workspace/Quinn-proof advisory audits pass |
| New deterministic safety coverage | **16** added tests cover eligibility, real quota release, busy/cancelled operations, stale reservations, stable/longest-idle selection, pressure/deadline races, one-second wait and shared budgets |
| Authentication proofs | Debug and release each pass TLS **8/8**, Quinn **3/3**, UDP environment and pinned Quinn version checks |
| Global production cap | **Nine** labs, three rounds each over QUIC/TCP/plain TCP, all hold 1024 real sessions for at least 65 s; **27** newcomers echo and exactly **27** own idle clients reset |
| Global ownership/protection | All **8,928** sibling sessions survive; all **27** empty-Fang and **27** recent-owner probes reset; no second victim or cross-Fang selection |
| Local and release controls | All nine local-64 controls replace exactly one old client; all nine different-Fang release recoveries pass; **9,216** surviving-client echoes |
| Global resource controls | Saturated initiator descriptors **2065** (QUIC) or **2096** (TCP/plain), below 4096; every lab returns **16 → 16**; two-second idle CPU **0.0%**; zero final target streams/receiver authority; survival and private cleanup pass |
| Stage54 idle shares | **18 groups / six labs**, three rounds per transport, **42** exact idle replacements and all twelve smaller-share release-one recoveries |
| Stage54 providers | All **four** direct/routed paths; twelve idle-primary replacements; **128** committed resets with **zero** fallback payload replay; active fallback and primary recovery pass |
| Reserved-share/provider/migration compatibility | **12** share cases, **five** provider/activation cases and **21** hostname migration cases pass |
| Full local CI | **13/13** actual gate exits zero, including **68** Python unit tests and fresh advisory audit |

All **19** global/share/provider native labs use full 65-second idle
observations, drain targets/authority, return initiator descriptors to
**16 → 16**, preserve daemon survival and remove owned processes/private
Dens. In secure global-cap labs, receiver usage is 32 before replacement
and 33 only after explicit release/recovery, below its quota. These are
local functional checks, not throughput or comparative liveness evidence.

Full CI: `target/ci-local/20261122T182659Z/`, **355.1 s**. Evidence root:
`target/stage55-m2/`, including all 17 actual worker exits, commands,
matrix digests, scenario verification, source provenance and observation
of the overall manager exit 0. All **321** source hashes still match the
fixed candidate; result notes are documentation-only changes afterward.
These ignored evidence files are not release artifacts or private identities.

Native debug daemon SHA-256:
`73538f6d6fdbf00713b931a589427dace5c7c4dbaf3b399e03008e2486b71ae1`.
Lockfile SHA-256:
`411ef3b496dfd8c21f97eb2285d55bd0d12ae604436acfdfa16044e80a0d91d0`.
RustSec snapshot observed after the final suite:
`d1fe6ac3eba823a3515e849045f36dc34543885d`.

The secure baseline used the exact same admission fixture bytes with
`--expect baseline` before runtime changes; the fixed campaign uses
`--expect reclaimed`. A separate fixed-source smoke passed all three
transports. One development compilation failed because a cancellation-test
`unwrap_err` required a `Debug` implementation for `FangSlot`; inspecting
the error variant fixed that test helper without changing its assertion.
The expected red regression and development compile output remain separate
from the final campaign. No failed final worker was retried or existing
gate assertion relaxed.

Final verification passes all **87** local document links and whitespace
checks. Existing integration tests/gates, scripts, dependency/lock inputs,
workflows and release inputs remain unchanged from merged M1 `5ce2a54`.

### Remaining scope

The full-cap native portion here measures direct QUIC/TCP/plain owners
with plain fillers. Routed/provider checks above validate the existing
Stage54 scenarios below global exhaustion. Full-cap routed/provider/source,
membership and concurrent-storm combinations remain M3 work. The waiter
holds a local permit while awaiting its selected victim; it gains no global
permit until actual release and reacquisition. The global limit counts
admitted slots, not kernel backlog or temporarily accepted waiting sockets.

M3 retains broader global-pressure routing/provider, source, concurrency,
membership and resource regression from the reviewed proposal. M2 does
not establish universal admission or fairness: one useful victim can still
lose a cleanup/capacity race, and a legitimate idle application can reset.
Native labs are local, with private generated identities and owned cleanup.
Frozen rc.1/rc.2, S3-P2/S2-P4 exclusions, Stage38 closure, Tokio 1.52.3 and
advisory exceptions remain unchanged; no installed node, real peer or WAN
host is used.
