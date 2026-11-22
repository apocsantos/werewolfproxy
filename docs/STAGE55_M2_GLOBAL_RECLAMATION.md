# Stage55 M2 — bounded global idle reclamation

**Status: implementation in validation, 2026-11-22.** The owner merged
[PR #174](https://github.com/apocsantos/werewolfproxy/pull/174) as
`5ce2a544975a9567ac5f48f3c71cadc65e361767` and instructed continuation.
The [reviewed proposal](STAGE55_GLOBAL_IDLE_ADMISSION_SURVEY.md) decisions
S55-D0–D6 are applied as recommended: G1, same Fang only, all Fang kinds.
This record covers M2; broader M3 regression and M4 closure remain separate.

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

The fixed-source campaign will record workspace format/Clippy/tests,
release build, advisory audits, debug/release authentication proofs,
three native rounds over QUIC/TCP/plain TCP, Stage54 idle/provider controls,
reserved-share and migration controls, and full local CI. Its final
results are recorded after completion, with source hashes and actual exits.

M3 retains broader global-pressure routing/provider, source, concurrency,
membership and resource regression from the reviewed proposal. M2 does
not establish universal admission or fairness: one useful victim can still
lose a cleanup/capacity race, and a legitimate idle application can reset.
Native labs are local, with private generated identities and owned cleanup.
Frozen rc.1/rc.2, S3-P2/S2-P4 exclusions, Stage38 closure, Tokio 1.52.3 and
advisory exceptions remain unchanged; no installed node, real peer or WAN
host is used.
