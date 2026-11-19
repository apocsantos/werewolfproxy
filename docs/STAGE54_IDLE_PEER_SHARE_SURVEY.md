# Stage54 — idle sessions at reserved peer shares

**Status: M1 reproduction and policy proposal for review, 2026-11-19.**
The owner authorised the next reproduction/design step after
[Stage53 closure](STAGE53_CLOSURE.md). This step changes only a disposable
fixture and documentation. No runtime policy is implemented or authorised
by the measurements alone.

## Baseline and concrete question

Baseline `main`: `8f86202c91a41b2758c5112e7d0e91dce8b2f3f0`, Stage53
closure merge PR #169. Tokio 1.52.3, quinn-proto 0.11.19 and quinn-udp
0.5.16 remain. Fixed reproduction fixture: `82e8487b933f9f4168494e5a0a7bee6c613d59eb`.

Stage53 reserves a local 64-session budget per captured receiver key:
two Fangs get 32 each and three get 22/21/21. Unused shares are not lent.
The existing Stage43 idle rule resets a longest-idle client after 60 s
when a newcomer hits a Fang's 64-slot local ceiling or an applicable
per-source ceiling. Peer-share exhaustion alone does not invoke it.

Thus a Fang with 32 or 21 idle sessions may deny a new client while
remaining below its local ceiling, even after every old session has been
idle long enough for Stage43 eviction. Stage53 recorded this boundary
from code. M1 now measures it and asks whether idle reclamation should
also respond to local peer-share pressure.

Relevant source paths:

- [Local admission and activity](../crates/werewolfd/src/transport/fang_accept.rs):
  `FangAcceptor::admit` handles local/source pressure; `FangSlot::serve`
  owns cancellation and `Tracked` records successful local payload I/O.
  Existing idle threshold is 60 s and eviction-release wait is 1 s.
- [Peer coordinator](../crates/werewolfd/src/transport/peer_quota.rs):
  `Membership::reserve` atomically checks the current attempt's keys and
  returns local `dial` on exhaustion. It neither queues nor evicts.
- [Provider selection](../crates/werewolfd/src/transport/failover.rs):
  reservation denial advances to the next provider before verified ACK;
  after ACK, target failure ends the client without provider replay.

## Disposable reproduction

[Tool and procedure](../tools/stage54/README.md). Each fresh five-node
lab has initiator A and receivers B/C/D/E. Independent receiver keys
isolate budgets while sharing one 65-second observation interval:

| Receiver | Members / owner share | Held owner clients | Other controls |
|---|---|---|---|
| B | One / 64 | 64 idle | Full local-ceiling positive control |
| C | Two / 32 | 32 idle | One active sibling client |
| D | Three / 21 | 21 idle, through the second-registered Fang | One active client per other sibling |
| E | Independent receiver | None | One active held client and three new clients |

All clients first echo. Three extra clients through each owner must reset
before the idle threshold. No owner connection then exchanges any bytes
for at least 65 monotonic seconds; sibling/independent clients exchange
payloads once per second. Targets have no application idle timeout.
All original owner sockets must remain open until the post-idle probe.

At 64, a new client must echo and exactly one old idle client must reset.
At 32/21, three new clients must reset as local `dial`, no new target
payload stream may appear, and every old client must remain open. Receiver
authority is only 33/23 including the sibling controls, so these cases
do not saturate the receiver's 64-session budget. Releasing one owner
session must restore admission without reopen; remaining held clients
must still echo. Final receiver authority is zero, target handlers drain,
descriptors return to baseline and all owned processes/private Dens close.

### Results

**CONFIRMED on 2026-11-18 in all 12 smaller-share cases**, three rounds
each over QUIC and encrypted TCP. All six full-ceiling positive controls pass. The tool
completed with actual exit **0**, with no failed run or assertion change.
Expected denial confirms a baseline limitation, not a passing fix.

| Observation | All three rounds, both transports |
|---|---|
| Idle interval | At least 65 monotonic seconds in each of six fresh labs |
| Share 64, after idle | New client echoes; exactly one old idle client resets, 63 survive |
| Share 32, after idle | Three newcomers reset as `dial`; all 32 old clients remain open; receiver has 33 sessions including sibling |
| Share 21, after idle | Three newcomers reset as `dial`; all 21 old clients remain open; receiver has 23 sessions including siblings |
| Smaller-share target growth on denial | Zero payload-bearing streams across all 36 post-idle probes |
| Explicitly release one smaller-share slot | All 12 recover without reopen; other held owner clients still echo |
| Active siblings / independent peer | Held echoes remain healthy; all 18 new independent-peer clients echo |
| Final receiver authority / descriptors | All active/establishing counts zero; initiator **16 → 16** in every lab |
| Survival / cleanup | All daemons survive; all private directories and owned processes removed |

All 54 pre-threshold probes reset as expected. Full-ceiling post-idle
clients echo in 0.0153–0.0551 s; smaller-share post-idle denials take
0.0002–0.0007 s. These are local probe observations, not comparative
latency, throughput or liveness certification. The fixture checks held
payload integrity and receiver authority as well as client outcomes.

Evidence root: `target/stage54-m1/`. `matrix.json` records all 18 groups,
client outcomes/times, authority snapshots, cleanup and all **313** source
input hashes, including the fixed tool and reused helpers. The separate
`matrix-verified.json` checks the scenario outcomes and matrix digest.
Daemon SHA-256 is
`7efcf89258c53c48988b5a36062497e54932deb256cc3d8152b6c7538ca26a58`;
lockfile SHA-256 is
`411ef3b496dfd8c21f97eb2285d55bd0d12ae604436acfdfa16044e80a0d91d0`.
All input hashes still match. These ignored evidence files are local
records, not committed release artifacts or private identities.

## Policy options

The proposed goal is to admit a newcomer by replacing one eligible idle
session in its own Fang when that replacement can satisfy the current
local peer-share reservation. It does not reclaim an active sibling's
share, lend unused shares or create capacity at a remote receiver.

| Option | Behaviour | Cost / boundary |
|---|---|---|
| I0 — retain Stage53 boundary | Document smaller-share denial; clients/operators release connections | No runtime change; idle clients can deny admission indefinitely |
| I1 — lower local ceiling to a computed share | Invoke existing local eviction at a smaller accept limit | A minimum across potential providers can block a healthy fallback or evict a session on an unrelated path; not recommended |
| I2 — attempt-scoped own-Fang reclamation | On local reservation denial, replace one eligible idle session that would make this attempt admissible | New bookkeeping/cancellation integration; an idle legitimate application may reset below the old 64-slot local ceiling |
| I3 — reclaim only after all providers fail | Prefer any usable fallback before considering idle replacement | Adds a second provider-selection phase and may have no deadline left; a different provider preference requiring its own contract |
| I4 — expire idle sessions automatically | End them after 60 s even without a newcomer | Broad session-lifetime change; unnecessary disruption without pressure |

Recommend **I2**, subject to the explicit decisions below. An active
session or an unrelated/sibling Fang must never be chosen merely to
rebalance shares. A legitimate idle SSH or database connection can be
reset under the proposed policy; this compatibility cost requires review.

## Proposed I2 contract — not implemented

### Eligibility and ownership

- Trigger only on a typed local peer-capacity denial for the current
  authenticated attempt. Remote refusal, resolver/handshake failure,
  poisoned/unavailable coordinator state and post-ACK target failure
  must not request reclamation.
- Consider only committed sessions belonging to this same Fang, with
  at least 60 s since the last observed successful payload I/O in either
  direction, using Stage43's monotonic activity semantics. Exclude the
  new client, setup work and sessions already marked for eviction.
  Recheck the sampled idle timestamp immediately before marking; activity
  may resume concurrently, as with existing pressure-driven idle eviction.
- Record each candidate's actual held receiver demand set, not the union
  of all potential providers. A candidate must release every currently
  blocking key needed by this attempt, and simulating its release must
  make all of this attempt's share and total checks pass. Choose the
  longest-idle eligible candidate; ties use a stable session identifier.
- Never evict several sessions to force a grandfathered Fang below its
  new share. If removing one cannot permit the attempt, do not evict.
  No sibling or other daemon's sessions are candidates.

### Reservation, cancellation and bounds

- Keep direct and routed reservations atomic across all demanded keys.
  Inspect eligibility/marking under short synchronous bookkeeping locks
  with one documented lock order; never hold a lock across cancellation,
  network work or an await. Do not decrement usage speculatively.
- Integrate with the owning `FangSlot` cancellation path. A victim's
  client gets a reset and its real reservation remains counted until
  owned cleanup releases it. Do not replay its established payload.
- Permit at most one victim and at most one reclamation operation per
  newcomer, sharing that budget with any earlier local/source eviction.
  Serialize peer reclamation within a Fang with nonblocking admission:
  simultaneous newcomers must not each select more victims. A busy
  reclamation gate follows ordinary local-denial/fallback handling.
- Wait at most the existing **1 s** eviction-release bound, also bounded
  by the remaining setup/selection deadline. Then recheck and acquire the
  current attempt's complete demand set once. No queue, borrowing,
  deadline restart or unbounded reservation retry loop. A race or slow
  cleanup may still deny the newcomer; do not promise admission.

### Providers, routing and compatibility

- Apply to direct QUIC/TCP, routed attempts and every failover plan by
  captured full receiver key. Plain TCP retains its existing local rule.
  Route/Pack generations and authorization remain captured as before.
- For failover, try eligible reclamation on the current provider before
  advancing to the next, within the existing selection deadline. This
  deliberately prefers the configured primary over an otherwise free
  fallback when one own-Fang idle primary session can be replaced.
  It changes the specific Stage53 capacity-fallback outcome for that
  eligible-idle case; active primary saturation still uses fallback.
- After verified ACK there is no provider retry or payload replay.
  Failure to reclaim/reserve follows existing provider order and setup
  error aggregation. Ordinary local capacity denial stays `dial` and
  uses the existing ten-second health/log limiter; eviction uses bounded
  existing-style logging, with no per-packet bookkeeping/logging added.
- Keep 64 local/per-peer, 1,024 daemon-local/global receiver and 64-member
  limits, source shares, resolver bounds, transport windows and all wire,
  persistent and CLI formats unchanged. No new operator setting in I2.

## Required review and milestones

| ID | Decision | Recommended |
|---|---|---|
| S54-D0 | Address the measured smaller-share idle boundary? | Yes, as pressure-driven own-Fang reclamation |
| S54-D1 | Policy I0–I4 | I2, accepting resets of legitimate idle clients below 64 |
| S54-D2 | Threshold and scope | Existing 60 s; same-Fang committed sessions and actual attempt keys only |
| S54-D3 | Reclamation bounds | One useful victim per newcomer, shared local/source budget, one operation per Fang, at most 1 s within existing deadlines |
| S54-D4 | Failover preference | Reclaim eligible idle capacity at the current provider before advancing; active saturation retains fallback |
| S54-D5 | Membership/over-share behaviour | No forced rebalance or multiple evictions; simulate one release, then recheck actual capacity |
| S54-D6 | Authorise implementation and regression? | Only after review of this proposal and its compatibility costs |

- **M1, this step:** retain baseline measurements and submit the contract.
- **M2 after review:** implement the selected policy, with tests for
  actual-plan key grouping, candidate selection, activity/commit marking,
  no useful victim, simultaneous newcomers, cancellation/release lifetime,
  lock ordering, deadline bounds and no double eviction across triggers.
- **M3:** repeat the 64/32/21 matrix with selected-policy expectations;
  cover 22-slot remainder, mixed transports, active versus idle sessions,
  active/recent-byte protection, non-loopback source limits, half-close,
  late over-share activation, membership shrink/growth during wait,
  common-relay/destination pressure, unused alternatives, direct/routed
  provider preference, slow cleanup, concurrent storms and post-ACK resets.
  Verify identity/policy, receiver/descriptor cleanup and bounded logs/CPU.
  Run workspace fmt/Clippy/tests, debug/release Stage12 proofs, full local
  CI, Stage42/43 pressure/fairness, Stage46/49 address/refusal, Stage52
  resolver/migration, Stage53 membership/providers and S2/S3/S4/S7 P0.
- **M4:** update limitations/status and close only from reviewed results.

M1 measures direct literal-IPv4 Fangs only. Routed/failover reclamation,
activity races and the proposed policy are design requirements, not
measured fixes. No per-user fairness or service guarantee during active
saturation follows. Frozen rc.1/rc.2, S3-P2/S2-P4 exclusions, Stage38
closure, Tokio 1.52.3 and advisory exceptions remain unchanged. No real
peer, installed node, service unit or WAN host is used. Comparative
liveness, a new release and hosted billing changes remain outside scope.

## M1 validation

Full disposable local CI: **13/13 PASS**, all actual gate exits and the
overall process exit 0, at `target/ci-local/20261119T103224Z/summary.tsv`,
including all **68 Python unit tests** and the workspace advisory audit.
The fixed tool and three documentation files remained unchanged throughout
CI; this final results note followed afterward. Python syntax/CLI help,
all **80** local document links checked and whitespace checks pass. All
**313** matrix source-input hashes still match the fixed fixture.

The committed changes are the new reproduction, its README, this survey
and an index link. Existing runtime/tests/gate definitions, dependencies,
advisory policy, workflows and release inputs remain unchanged from
`8f86202`. No new workspace Rust, standalone proof or chaos campaign is
claimed for M1; those checks follow implementation of a reviewed policy.
