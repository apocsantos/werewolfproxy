# Stage31 automatic governance sync — behaviour spec

**Status: approved 2026-10-27 (S1–S5 as recommended; see
[Spec decisions](#spec-decisions)); steps 1–2 done (real-host check
[below](#real-host-check-2026-10-27)), see
[Implementation](#implementation-after-approval).** Corrections found while
implementing are marked *Corrected in step 1*. This implements the
accepted decisions of the [Stage31 survey](STAGE31_AUTO_SYNC_SURVEY.md#decisions-accepted-2026-10-26)
(1a, 3a, 6a and 7a as corrected by the owner; 2a, 4a, 5a, 8a and 9). It
changes when the Stage29 `governance sync` runs, and nothing about what it
does: the [wire spec](STAGE29_GOVERNANCE_RPC_WIRE_SPEC.md) (G1–G3), the
statement format, the threshold, the archive and the fork fence are
unchanged byte for byte.

## Terms

- **Run:** one automatic sync: the network phase of manual sync
  (`resolve_sync`, every Pack member, `--transport quic`), then, if it
  found anything, one `governance.sync_apply` through mutation admission.
- **Resolving / applying:** the two phases of a run. Resolving holds the
  discovery lock and only sends read-only RPCs; applying starts when the
  run submits `governance.sync_apply` (the lock is already released, as in
  manual sync today).
- **Eligible:** the predicate in section 1.

## 1. Eligibility

A node is eligible exactly when all of these hold:

1. it has a Pelt and its secure listeners are configured (`pelt_ready`);
2. it has a governance head with `activated_at` set, `dead_at` unset and
   `forked` false;
3. it is admitted (`!governance::unadmitted`);
4. the switch (section 2) is on;
5. Silver is not active;
6. it is not DYING;
7. its storage is not degraded (`storage_degraded` false; a run could not
   commit anyway).

Before activation, during the ceremony and on an unadmitted node, sync
stays manual and the Admission arrives by file (decision 1a, corrected).

## 2. The switch

- **Command:** `werewolfctl governance auto-sync on|off`, control command
  `governance.auto_sync` with `{"state": "on"|"off"}` (*corrected in step
  1:* every control argument is a string, so the draft's boolean
  `{"enabled": …}` could not pass the control parser; anything else is
  `AUTO_SYNC_INVALID`). A mutation: it
  goes through mutation admission, is refused while DYING, and is allowed
  in every other state (including before a genesis), so an operator can
  turn it off before the node ever becomes eligible.
- **Storage:** independent Den state (ARCHITECTURE §8.2), file
  `auto_sync.json`, through the Den's atomic replace:

  ```json
  {"version":1,"enabled":false}
  ```

  Exactly these two fields, in this order, both values written explicitly.
  Absent file: enabled (decision 2a, default on). Any other content fails
  the start closed, like the other Den documents.
- **Order:** persisted before the runtime flag changes ("persist before
  publishing runtime state"). Outcomes:
  - durably committed: the flag changes; the result is
    `{"enabled": …, "status": "saved"}`;
  - not committed: nothing changes, `AUTO_SYNC_SAVE_FAILED`;
  - indeterminate after rename: `storage_degraded` is set, as for every
    other Den write, and `AUTO_SYNC_SAVE_FAILED` is returned.
- Turning it off cancels a resolving run and any timer; it never cancels
  an applying run (section 4).
- Destruction (Stage30) removes the file with the rest of the Den.

**Spec decision S1** (below) is whether this file belongs in the
security-state manifest instead.

## 3. The schedule

One scheduler task per daemon, owned by the control server and stopped
with it (by `shutdown_owned_tasks`, which the terminal transition also
reaches); stopping it aborts a resolving run. All
times use the monotonic clock (`tokio::time::Instant`); wall-clock shifts
never move a deadline.

| From | Event | To |
|---|---|---|
| ineligible | becomes eligible (section 1) | waiting, due in U(5 s, 30 s) |
| waiting | due | resolving |
| waiting | becomes ineligible | ineligible (timer dropped) |
| resolving | finishes with nothing to apply | waiting, due in U(450 s, 1350 s) |
| resolving | finishes with statements or a conflict | applying |
| resolving | becomes ineligible, or an operator command (section 4) | waiting, due in U(450 s, 1350 s), or ineligible |
| resolving | the discovery lock is busy at start | waiting, due in U(450 s, 1350 s) |
| applying | the mutation returns | waiting, due in U(450 s, 1350 s), or ineligible |

- U(a, b) is uniform over [a, b], drawn from the OS RNG. 450–1350 s is
  the accepted 15 minutes ±50%.
- **Initial run on every ineligible → eligible transition** (decision 3a,
  corrected), not only at start-up. Eligibility is re-evaluated after
  every committed mutation, after Silver changes and after the switch
  changes, so these all count: start-up of an eligible node; applying the
  activation on an admitted node; applying the node's own Admission on an
  active chain; the switch turned on; Silver cleared. (*Corrected in step
  1:* degraded storage never recovers in a running daemon, only at the
  next start, which is a start-up transition.)
- **No overlap:** one scheduler, one run at a time. A run never queues
  behind another discovery; if the lock is busy it is skipped.
- A run that ends with the node no longer eligible (for example it applied
  a revocation of itself, a fork fence, or a death) goes to ineligible.

## 4. Operator precedence (decision 6a, corrected)

- The operator commands that take the discovery lock are `discover
  refresh`, `discover find` and `governance sync` (*corrected in step 1:*
  the draft listed `discover services`, which takes no lock, and missed
  `discover refresh`).
- Such a command first tries the lock; only if it is held by an automatic
  run that is resolving does it cancel that run, and it then waits at most
  2 s for the lock. A lock held by anything else is `DISCOVER_BUSY`, as
  before.
- If an automatic run is **resolving**, such a command cancels it and then
  takes the lock: the run's network task is aborted and its guard dropped.
  Resolving only sends read-only RPCs and changes no state on either side,
  so cancellation has no effect to undo. The operator command then
  proceeds as today.
- Once a run has submitted `governance.sync_apply` (**applying**), it is
  never cancelled. It no longer holds the discovery lock, so operator
  commands are not blocked by it; the apply itself re-verifies every
  statement against the live head and is serialized with other mutations
  by mutation admission, as manual sync is today.
- An automatic run never cancels an operator command; if the lock is held
  by one, the run is skipped (section 3).

## 5. What a run applies (decisions 5a and 7a)

Exactly what manual sync applies: every statement kind, re-verified with
seven governance signatures against the live head, archive before commit
(G3). Two different valid successors fence the node (`forked`), which
makes it ineligible.

**Death (7a, clarified).** A compliant responder never serves a death:
it applies a death only by destroying itself first. The death file stays
the certified delivery path. There is no requester-side weakening: if any
responder does present a valid death (seven signatures, chaining from the
live head), the requester applies it like any other statement and
performs its Stage30 terminal transition, from an automatic run as from a
manual one. Nothing in this spec filters statement kinds.

## 6. Status and logs (decision 8a)

`governance status` gains:

```json
"auto_sync": {
  "enabled": true,
  "eligible": true,
  "ineligible_reason": null,
  "phase": "waiting",
  "next_due_in_secs": 812,
  "last_run": {
    "started_secs_ago": 95,
    "outcome": "nothing_new",
    "applied": [],
    "peers": [{"peer": "node-c", "status": "ok", "offered": [0]}]
  }
}
```

- `ineligible_reason`: the first failing condition of section 1, one of
  `no_pelt`, `not_ready` (listeners not yet configured), `no_genesis`, `not_activated`, `dead`, `forked`,
  `unadmitted`, `switched_off`, `silver`, `dying`, `storage_degraded`;
  `null` when eligible.
- `phase`: `ineligible`, `waiting`, `resolving` or `applying`;
  `next_due_in_secs` is `null` unless waiting.
- `last_run.outcome`: `applied`, `nothing_new`, `stopped` (with the
  `sync_apply` stop code), `fork`, `skipped_busy` or `cancelled`;
  `last_run` is `null` until the first run. Times are whole seconds from
  the monotonic clock.
- All of it is runtime only; only the switch is persisted.
- `status` (the general command) is unchanged; its schema is pinned by the
  JSON schema gate.
- Logs: one line per run outcome other than `nothing_new`
  (`[GOVERNANCE][AUTO_SYNC]`), and one line per applied statement
  (`[GOVERNANCE][AUTO_SYNC_APPLIED]` followed by its apply result as JSON:
  sequence, hash and effects such as `revoked_members` and
  `closed_fangs`).

## 7. Lab timing

For the lab gate only, the `lab-faults` build feature (Stage30) also
reads:

- `WEREWOLF_LAB_AUTO_SYNC_FIRST_MS`: replaces U(5 s, 30 s);
- `WEREWOLF_LAB_AUTO_SYNC_INTERVAL_MS`: replaces U(450 s, 1350 s).

Release builds compile neither in. Each lab value is used as an exact
delay, so the gate can predict runs.

## 8. Load

A run sends each Pack member at most one `governance` request per round,
at most 16 rounds; a typical run is one round. The responder's per-peer
token bucket (burst 16, 4 per second) is therefore never exhausted by one
requester's automatic runs, even back to back, and operator RPCs keep
their share.

## 9. Errors and codes

| Code | When |
|---|---|
| `AUTO_SYNC_INVALID` | the switch command without `state` `on` or `off` |
| `AUTO_SYNC_SAVE_FAILED` | the switch could not be saved (section 2) |
| `LIFECYCLE_DYING` | the switch command while DYING (existing refusal) |

Automatic runs answer no client, so their failures appear only in
`auto_sync.last_run` and the log.

## 10. Amendments

- **Stage29 wire spec, G4:** annotated "superseded for active, admitted
  nodes by Stage31"; manual sync is unchanged and stays available.
- **ARCHITECTURE §30:** governance propagation joins the autonomous
  operations, with the death boundary of section 5.
- **V1 CLI contract:** `governance auto-sync on|off` and the
  `auto_sync` status object.

## 11. Tests required before merge

Unit:

- the switch file: strict parse, absent means on, malformed fails the
  start; persistence order and each `CommitOutcome`;
- eligibility: each condition of section 1 alone makes the node
  ineligible, with its `ineligible_reason`;
- the scheduler, on a paused tokio clock: bounds of both delays; an
  initial run after each ineligible → eligible transition; no overlap;
  skip when busy; cancellation while resolving and never while applying;
  a wall-clock change moves nothing;
- a responder test double that serves a valid death: the requester dies
  (7a).

Lab gate `tests/integration/auto_sync_gate.py` (lab timing):

- a revocation applied by file on one node reaches every active, admitted
  node with no operator command, and the revoked member's Fangs close;
- ceremony nodes and an unadmitted node never sync on their own; the
  unadmitted node, once given its Admission by file, starts automatically;
- the switch survives a restart and stops all automatic RPCs (checked on
  the responder's side);
- Silver, DYING and a fenced fork stop it; a fork found by an automatic
  run fences and stops further runs;
- an operator `governance sync` during a resolving run wins at once;
- nothing is applied twice; archives stay byte-identical to the files;
- a responder never rate-limits automatic runs.

Real-host check (step 3) on disposable daemons only: a revocation
propagating over the WAN through the `node-b` relay with no command on the
receiving node.

## Spec decisions

All five approved by the owner on 2026-10-27 as recommended.

- **S1. Where the switch lives.** Recommended: independent Den state
  (`auto_sync.json`). The switch grants no authority and only changes when
  a statement arrives, never whether it applies; every statement still
  applies by file or manual sync. The alternative is the security-state
  manifest, which detects a rollback of the switch together with the rest,
  at the cost of a manifest format change and a migration for every
  existing node.
- **S2. The delays.** Recommended: U(5 s, 30 s) for the initial run and
  U(450 s, 1350 s) for the interval, as above.
- **S3. A busy skip waits the full interval.** Recommended: yes. The lock
  is busy because an operator is running discovery or sync, so a sync is
  happening anyway; retrying sooner adds traffic for little gain.
- **S4. Degraded storage makes a node ineligible.** Recommended: yes; a
  run could not commit, and asking peers would be traffic with no effect.
- **S5. Lab timing under `lab-faults`.** Recommended: reuse the existing
  lab-only feature rather than add another; it is already excluded from
  release builds.

## Implementation (after approval)

| Step | Scope | State |
|---|---|---|
| 1 | Daemon: the switch and its file, the scheduler, operator precedence, status, logs; unit tests; lab gate `auto_sync_gate.py` | Done |
| 2 | Real-host check on disposable daemons only | Done, [below](#real-host-check-2026-10-27) |

Step 1 notes:

- **Where the tests stand against section 11.**
  - The scheduler tests run on the real clock with short fixed delays (a
    test-only override), not a paused clock: `tokio`'s `test-util` feature
    is not enabled, and adding a dev-dependency only for this was not
    worth it. The delay bounds have their own test. "A wall-clock change
    moves nothing" holds by construction: every deadline is a
    `tokio::time::Instant`, which never reads the wall clock; it is not
    tested separately.
  - The 7a death test drives `governance.sync_apply`, the requester's only
    entry point for sync results, with a valid death; no responder double
    exists for the `governance` RPC, and the RPC layer does not look at
    statement kinds.
  - DYING is not exercised in the lab gate (it lasts one terminal
    transition); the unit tests cover it as an ineligibility and as a
    refusal of the switch command.
  - "The responder's side" in the gate is a UDP sink standing in for a
    Pack member (`wolf-trap`, its key admitted): it counts the QUIC
    datagrams the node sends to it. Because the sink never answers, a run
    that includes it stays resolving for the 7 s lookup budget, which is
    also how the gate catches a run resolving for the operator check.
- **A revoked node cannot learn its own revocation by sync (finding).**
  Its former peers applied the revocation, which removed it from their
  Packs, so they refuse its requests (`rejected`). It keeps its last head
  and stays eligible as far as it knows, running and being refused, until
  its operator applies the revocation file (it then becomes unadmitted and
  stops) or turns the switch off. This is the Stage29 behaviour of manual
  sync as well; the gate checks it.
- **The control parser accepts only string arguments**, which is why the
  switch is `{"state": "on"|"off"}`; a parser test now pins it, since the
  unit test that calls the handler directly could not catch it.

## Real-host check (2026-10-27)

Build `3aa7f5c` (main after #74), release, `--locked`: `werewolfd`
`266171be…cd7d`, `werewolfctl` `2c6b46c6…e914`, hash-checked on both hosts.
No lab timing: D ran the release delays (U(5–30 s), then U(450–1350 s)).
Only disposable daemons held governance state:

| Node | Host | Role |
|---|---|---|
| D | DarkSide, user systemd unit, loopback, fresh Den and Pelt (`wwp1:1A-DB-D5-F4-75-93-D8-F7`) | the receiver: after its activation, only read-only `governance status` (one exception, below) |
| C | VPS, user mode, loopback only (`127.0.0.1:18463`), fresh Den and Pelt (`wwp1:9F-2A-D0-22-58-18-33-37`) | serves; every statement by file |
| `node-b` | VPS `werewolfd.service`, `bcf7bbd` | Stage26 relay only, unchanged |

D reached C only through `node-b` (route `node-c --via node-b`, QUIC hop).
The owner's conditions for the production relay were applied as in
Stage29 and Stage30 (lists recorded before; `node-d` with D's exact key;
one grant, `127.0.0.1:18463`; nothing else changed; cleanup removed the
grant, then `node-d`; both lists diffed `IDENTICAL`), plus one the owner
accepted for this check: automatic sync asks every Pack member, so each of
D's runs also sent `node-b` one read-only `governance` request. `node-b`'s
build has no such RPC and refused each one at field validation
(`rejected`), changing nothing.

Ten fresh governance keys stayed on DarkSide and were deleted afterwards;
only signed statement files went to the VPS. The chain, verified offline
with `inspect --chain --expect-genesis` (network `dc42d57b…cdff`):

| Seq | Statement | Hash | C got it | D got it |
|---|---|---|---|---|
| 0 | genesis | `a5a72030…c1a2` | file, `--expect-hash` | file, `--expect-hash` |
| 1 | admit C, D, `node-b`, `ghost` (a dummy key in D's Pack) | `798a937b…d7fe` | file | file (ceremony stays manual) |
| 2 | activate | `79a7008d…5db3` | file | file |
| 3 | revoke `ghost` | `951a90ee…cdfee` | file | **automatic sync** |

| Check (UTC) | Result |
|---|---|
| A, 14:23:30 | PASS: D activated by file; its first automatic run started about 24 s later (scheduled 22 s): `nothing_new`; `node-c` `ok` through `node-b`; `node-b` `rejected`; `ghost` `bad_address` (a placeholder TCP address); next run due in 614 s |
| B, 15:07:10 | PASS: the revocation applied on C by file at about 15:01; D's next scheduled run, with no command on D, fetched it through `node-b` (`node-c` `ok`, offering `[1]`) and applied it: journal `AUTO_SYNC_APPLIED` with `revoked_members: ["ghost"]`, sequence 3, hash `951a90ee…`; D's archive entry 3 byte-identical to the file; D's Pack `node-b`, `node-c` |
| C, 15:07:40 | PASS: `systemctl --user restart`: D started (`NRestarts=0`) and ran again on its own about 8 s later: `nothing_new`, still sequence 3; `governance status` apart from `auto_sync` identical before and after |

Notes:

- **The first attempt at B** produced no change on D because the
  revocation never reached C: the first apply was typed on DarkSide, where
  no C exists, and C itself had stopped with the SSH session it ran in
  (about 14:55, after D's 14:53 run had reached it). D's runs at about
  14:34 and 14:53 both reported `node-c` `ok` offering `[0]`, which is how
  the gap showed. C was restarted detached from its unchanged Den (still
  at sequence 2), and the revocation was applied on it at about 15:01.
- **One command besides `governance status` reached D after its
  activation:** at about 14:54 a `pack add ghost`, sent in error while
  checking, was refused (`ghost` was still a member) and changed nothing.
  It preceded the revocation's arrival at 15:07 and has no bearing on B.
- At cleanup, `kill 19580` (C's QUIC port, typed as a PID) was run twice
  and answered "No such process"; C was then stopped by its exact PID.

Not shown on real hosts (covered by `auto_sync_gate.py` and unit tests):
operator precedence, the switch, Silver, a fork, an unadmitted node.

wolf-a, wolf-b and DarkSide's Stage22R daemon were not touched; on the VPS
the production `werewolfd` and `werewolf-c` were running before and after,
untouched; no governance statement reached a real node.
