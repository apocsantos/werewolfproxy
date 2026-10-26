# Stage31 automatic governance sync — design survey

**Status: survey for owner decisions.** Docs only. The rules go into a
spec for review before any code, as in Stage28 and Stage30.

Stage29 decision G4 ([wire spec](STAGE29_GOVERNANCE_RPC_WIRE_SPEC.md#decisions-approved-2026-10-26))
kept sync manual: wire protocol, archive semantics, fork detection and
manual convergence first, because "automatic sync would make governance
propagation a permanent property of the system rather than a deliberate
action". Those are now done and checked on real hosts (Stage29 WAN check,
Stage30 step 3). [ARCHITECTURE.md](ARCHITECTURE.md) §29–30 sets the goal:
privileged bootstrap, then an active network that runs revocation,
policy updates and lifecycle changes without a root login on each node.

This survey answers nine questions:

1. when automatic sync runs at all;
2. how an operator controls it on one node;
3. what starts a sync and how often;
4. which peers each run asks;
5. which statements it applies without an operator;
6. how it shares the node with operator commands;
7. what it means for death;
8. what an operator can see;
9. how it is tested.

## What exists today

- **No autonomous network activity.** `werewolfd` never contacts another
  node on its own. Discovery, `services`, `find_node` and `governance sync`
  run only when an operator asks; the only timers are the QUIC and TCP
  keepalives of Fangs that already exist. Automatic sync would be the
  first behaviour the daemon starts by itself. That is the main reason this
  is a decision and not a feature.
- **Manual sync** (`governance sync [--peer] [--transport]`), resolved
  outside mutation admission in `discovery/operator.rs::resolve_sync`:
  - refuses on Silver, no Pelt, no genesis, a fenced fork, DYING, or when
    another discovery holds the discovery lock (`DISCOVER_BUSY`);
  - asks every Pack member (directly or through its route) for the
    statements after the simulated head, up to 16 rounds, each round
    bounded by a 7 s budget;
  - two different valid successors fence the node before either applies;
  - applies the chosen statements through the internal mutation
    `governance.sync_apply`, which re-verifies each one against the live
    head (seven signatures, chaining, archive before commit, G3).
- **The responder** serves only statements it has applied, at most four
  per request (G2), positioned by head hash (G1), under the shared per-peer
  RPC token bucket (burst 16, 4 per second). A Silver- or admission-locked
  responder answers nothing.
- **Effects of applying** are the same from a file or from sync:
  admission unlocks a node, revocation removes members and closes their
  Fangs, rotation replaces governance keys, activation sweeps non-roster
  members from the Pack.

Nothing here changes the wire, the transcripts, the statement format, the
threshold or any trust rule. Automatic sync only decides **when** the
existing pull runs, without an operator.

## 1. When it runs at all

**1a. After activation only (recommended).** Automatic sync runs only on
a node whose chain is `active`, including an activated node that is not
yet admitted (it pulls the admission that unlocks it). During the
ceremony (genesis applied, not activated) and before any genesis, sync
stays manual.

- Matches ARCHITECTURE §29–30: the ceremony is the deliberate,
  operator-driven phase; after activation the network runs itself.
- The activation statement, which sweeps each node's Pack, still reaches
  every node through a deliberate action (a file or a manual sync), as in
  the Stage29 WAN check.
- Cost: a node offline at activation stays in the ceremony until its
  operator acts. It enforces no roster meanwhile, so nothing is weakened.

**1b. Whenever a genesis exists.** Ceremony nodes also pull admissions and
the activation automatically. Faster setup, but the activation sweep then
happens on nodes without anyone choosing the moment.

**1c. Opt-in per node.** Off unless the operator enables it on that node.
Keeps G4's "deliberate action" fully, but a revocation then reaches only
nodes whose operators remembered to opt in, which is the gap auto-sync
exists to close.

## 2. Local control

**2a. A persisted local off switch (recommended).**
`governance auto-sync off|on`, saved in the Den before it takes effect
("persist before publishing runtime state"), surviving restarts. Default
`on` under 1a. Off means exactly today's behaviour.

- An operator can stop autonomous traffic on one node (for example while
  diagnosing it) without leaving the network.
- Turning it off never removes the node from governance: a revocation
  still applies the next time sync runs manually or a file is applied.

**2b. No switch.** Simpler, but the first autonomous behaviour would have
no local stop short of Silver.

Either way, Silver, DYING, a fenced fork and a missing Pelt stop automatic
sync exactly as they refuse a manual one.

## 3. What starts a run, and how often

**3a. Once after start, then a jittered interval (recommended).**

- One run shortly after the node reaches `READY` (a short random delay, so
  a fleet restarted together does not ask at once).
- Then one run per interval: a base interval (proposal: 15 minutes) drawn
  uniformly within ±50%, measured on the monotonic clock, so clock shifts
  (a later robustness test) cannot stall or burst it.
- A run never overlaps another; a run that finds the discovery lock busy
  is skipped, not queued.

**3b. Event-driven only:** run when a Fang to a Pack member comes up. No
idle traffic, but a node with no new Fangs never learns of a revocation.

**3c. 3a plus 3b.** Lower latency after reconnects, more cases to test.
Could be added later without a new decision on the rest.

## 4. Which peers each run asks

**4a. Every Pack member, as manual sync does (recommended).** The run is
literally the manual resolver with no `--peer`, so there is one code path
to certify. With Packs of a few to a few dozen members and a 15-minute
interval, the load is a handful of small RPCs per interval, far under the
responder's token bucket. Unreachable members cost only their share of the
7 s round budget.

**4b. One member per run, chosen at random.** Less traffic per run and a
smaller burst, but slower convergence and a second code path. Worth it
only for large Packs, which Werewolf does not have.

Transport: the manual default (`--transport quic`). No new transport
rule.

## 5. What it applies without an operator

**5a. Every statement kind, exactly as manual sync (recommended).**
Admission, revocation, rotation (and activation, which under 1a never
arrives automatically). The authority is the 7-of-10 threshold, not the
local operator; letting one operator delay a network-wide revocation by
not acting would recreate the per-node superuser that ARCHITECTURE §4.4
removes.

**5b. Fetch and verify automatically, apply only by operator.** The node
reports a pending verified statement. Keeps a human in the loop, but a
revoked member keeps its access on every node whose operator is away,
which defeats the purpose.

**5c. Apply revocations automatically, hold the rest.** A middle ground;
it splits the kinds into two classes that must each be certified, for
little gain under 1a.

Under every option, a fork (two valid, different successors) fences the
node exactly as manual sync does, and automatic sync then stops until the
fork is resolved by hand.

## 6. Sharing the node with operator commands

Both share the discovery lock. A full manual-style run can last up to
16 rounds × 7 s in the worst case (every member slow or unreachable).

**6a. Operator first (recommended).** An operator discovery command
(`discover`, `service add --from`, `governance sync`) cancels an
in-flight automatic run before its apply step and proceeds; the apply
itself (`governance.sync_apply`) is one mutation and is never cut in half.
The next automatic run starts from the then-current head.

**6b. First come, first served.** The operator may get `DISCOVER_BUSY` for
up to about two minutes. Simpler, but the operator loses to a background
task on their own node.

## 7. Death

Unchanged and restated: **automatic sync cannot deliver death.** A
responder serves only applied statements, and a node that applies a death
destroys itself before answering anything (Stage30 spec, step 2 notes;
checked on real hosts in Stage30 step 3). The death statement file stays
the only channel. A node whose peers have all died sees them as
unreachable; automatic sync does not change that.

- **7a. Accept (recommended).** The death runbook keeps listing every node
  and its file delivery.
- **7b. Reopen two-phase death** (Stage30 survey 6b: a DYING node serving
  the death for a bounded window). Already rejected for Stage30; listed
  only so the boundary is a recorded decision, not an accident.

The Stage30 survey's "Then" list still says automatic sync "will
propagate death as an ordinary statement"; that line is corrected
alongside this survey.

## 8. What an operator can see

**8a (recommended).** `governance status` gains a runtime-only object:

- `auto_sync.enabled` (the persisted switch) and `auto_sync.active`
  (whether 1a's condition holds now);
- the seconds since the last run started and until the next is due
  (monotonic);
- the last run's result: statements applied, per-peer statuses as manual
  sync reports them, or why it was skipped (busy, Silver, forked).

Each automatically applied statement is logged with its sequence and
hash. Nothing of this is persisted except the switch.

## 9. Testing

- **Lab gate** `auto_sync_gate.py`, with the interval shortened only by a
  lab-only build feature (as `lab-faults` in Stage30), never in a release
  build:
  - a revocation applied by file on one node reaches every activated
    node with no operator command, and the revoked member's Fangs close;
  - an unadmitted activated node is unlocked by its admission;
  - ceremony nodes never sync on their own (1a);
  - the off switch survives a restart and stops all automatic RPCs;
  - Silver, DYING and a fenced fork stop it; a fork found by automatic
    sync fences and stops further runs;
  - an operator command during an automatic run wins (6a);
  - nothing is applied twice; archives stay byte-identical;
  - a responder's token bucket is never exhausted by automatic runs.
- **Unit tests:** the scheduler (jitter bounds, monotonic clock, no
  overlap, skip on busy), and the status object.
- **Real-host check** on disposable daemons only, as before: a revocation
  propagating over the WAN through the `node-b` relay with no command on
  the receiving node.

## Security notes

- No wire, transcript, statement, threshold or trust change. A responder
  can still withhold statements but never forge one; automatic sync only
  asks more often.
- **Traffic pattern.** Periodic RPCs to every Pack member are a new,
  observable rhythm. Jitter blurs the period; the off switch (2a) and
  Silver remove it. Whether this matters for the Hide modes is for the
  owner to judge; it is noted, not solved, here.
- A malicious Pack member can at most fence a node by presenting a
  conflicting statement, and only with seven valid governance signatures,
  which is a governance compromise in itself; fencing is the intended
  response, as for manual sync.

## Plan (after decisions)

| Step | Scope |
|---|---|
| 0 | This survey; owner decisions 1–9 |
| 1 | Spec for review: the switch and its storage, scheduler constants, cancellation rule, status fields, error codes |
| 2 | Daemon: the scheduler, the switch, cancellation, status; lab gate `auto_sync_gate.py` |
| 3 | Real-host check on disposable daemons only |

Then, in the owner's order: robustness and chaos certification (netem
loss, jitter and reordering, clock shifts), then the CLI
architecture-complete candidate.

## Recommendations in one line

1a, 2a, 3a, 4a, 5a, 6a, 7a, 8a, and the tests in 9.
