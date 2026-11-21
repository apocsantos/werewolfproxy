# Stage55 — idle admission at the daemon-wide Fang ceiling

**Status: M1 reproduction and policy proposal in validation, 2026-11-21.**
After owner-merged [Stage54 closure](STAGE54_CLOSURE.md), the next bounded
inspection found a separate admission boundary. Baseline `main`:
`9b216649d706ea70288717ff48ed15a320d49a1e` (PR #173). This step adds a
disposable reproduction and documentation; runtime, existing tests/gates,
dependencies, workflows and release inputs remain unchanged. No new
eviction policy is implemented or authorised by measurements alone.

## Code facts and question

The shared [Fang acceptor](../crates/werewolfd/src/transport/fang_accept.rs)
enforces 64 local slots per Fang and **1024** across the daemon. Its order is:

1. Apply the non-loopback source share and, when applicable, its idle rule.
2. Acquire a local Fang slot; if full, invoke existing idle replacement.
3. Try the global permit once. If unavailable, reset before creating a
   session entry or starting transport/provider setup.

Stage54 peer reclamation happens later, within an authenticated attempt
that already owns its local/global slots. It cannot run when step 3 fails.
The global rejection path itself never looks for an own-Fang idle victim.
This ordering is **CONFIRMED from source** for the shared acceptor used by
plain, encrypted-TCP, QUIC, routed and failover Fangs. Existing unit tests
check global sharing and release, but not idle replacement below the local
ceiling at a saturated global cap.

Consequently, a Fang holding 32 idle sessions can deny newcomers when other
Fangs fill the daemon's remaining slots, even though replacing one of its
own idle sessions could free global capacity. A 64-session Fang can invoke
its local idle rule first and recover in the same globally saturated lab.
This is an **operational policy limitation**, not a cap bypass or a confirmed
security vulnerability. Active/global saturation remains bounded as intended.

The question is whether global pressure should also permit one bounded
replacement of an established idle session in the same Fang. Stage54 did
not approve that trigger; this is a separate compatibility decision.

## Disposable reproduction and evidence levels

[Fixture and invocation](../tools/stage55/README.md). One initiator owns
thirty plain-TCP Fangs with 32 sessions each and one with 64: exactly
**1024** echoed sessions, all below the production 4096-descriptor limit.
An additional empty Fang tests global denial without an own victim.
One selector-based echo loop avoids per-target threads and has no idle
timeout. All paths use dynamic literal loopback addresses and private
generated Dens; there is no host-network or installed-node change.

Every old client must remain open after at least **65 monotonic seconds**
without payload. Three 32-owner probes and three empty-Fang probes must
reset with zero target growth and no old eviction. The 64-owner positive
control must admit one newcomer and reset exactly one old client, with
all other Fangs preserved. Its replacement keeps the global total full;
the 32-owner must still deny a new probe. Releasing a different Fang's
session must then restore that owner's admission without another reset.
All 1024 surviving clients echo before complete cleanup and descriptor drain.

| Claim | Evidence level and boundary |
|---|---|
| Global denial precedes authenticated peer reclamation | Confirmed source ordering in the shared acceptor and transport callers |
| Plain-TCP idle global denial with production caps | Native reproduction; final three-round results recorded after validation |
| Target health, existing local idle eviction and release recovery | Native positive controls in the same saturated lab |
| Equivalent full-cap secure/routed/failover outcomes | Supported by shared code; **not natively measured by this matrix** |
| New global-trigger policy is correct or universally admits clients | Not established; requires review, implementation and regression |

The smoke run reproduced the expected boundary and passed cleanup. The
fresh fixed-source matrix and full local CI are separate final validation.

## Policy choices

| Option | Behaviour and tradeoff |
|---|---|
| G0 — retain the current global rule | Document immediate denial until some session releases global capacity; no new reset compatibility cost |
| G1 — bounded own-Fang global replacement | At global denial, replace one useful established own-Fang idle session; preserves ownership but may reset legitimate idle applications below 64 slots |
| G2 — evict across Fangs | Can serve an empty newcomer Fang but lets it disrupt siblings; broad fairness/ownership change, not recommended |
| G3 — repartition global slots among Fangs | New membership/utilisation policy and late-activation rules; substantially broader than this boundary, not recommended |

Recommend **G1** if the owner wants to address the measured boundary.
G0 remains a valid explicit choice. G1 should retain these constraints:

- Trigger on actual current global exhaustion, with local admission room;
  recheck pressure before marking so an already-free slot does not cause
  an unnecessary eviction. Never borrow another Fang's session.
- Use the existing **60 s** successful-payload activity threshold and
  longest-idle ordering, with stable ties. Exclude the newcomer, setup work,
  recently active and already-evicting entries. For secure sessions,
  established means final destination ACK verified; for plain TCP it means
  successful target connection. Plain TCP currently lacks peer-reservation
  metadata, so eligibility needs explicit representation rather than
  treating an absent reservation as committed.
- Keep the production **64 / 1024 / 16** limits and one victim per newcomer
  across local, source, global and peer triggers and all providers. A prior
  victim consumes the budget; no second victim repairs an unrelated later
  denial. Do not force old members under a newly smaller peer share.
- Cancel through owned session cleanup; never credit a marked session as
  free. Wait for that victim's real release at most the existing **1 s**,
  then try acquisition once. No global semaphore queue or unbounded retry.
  Global release can be taken by a competing Fang; denial is allowed.
- Bound simultaneous reclamation within a Fang, preserve the registry →
  coordinator lock order, and never hold a bookkeeping lock across await.
  Share the operation/victim budget with existing Stage54 reclamation.
- Charge new global-release waiting to the original setup/dial/selection
  budget, with no restarted deadline or new timeout constant. M2 must
  specify how that budget is captured before session creation; unchanged
  local/source semantics and the existing TCP setup retry need controls.
- Preserve receiver authorization, captured route/identity demands,
  provider order and final-ACK commitment. A global victim can free local
  capacity without making the eventual provider's peer share fit; do not
  select another victim or replay an established session to compensate.
- Keep rate-limited admission logs and existing failure visibility; no
  operator setting, persistent/wire format, dependency or live-node change.

## Required decisions and milestones

| ID | Decision | Recommended |
|---|---|---|
| S55-D0 | Address the measured global idle boundary? | Yes, subject to the reset compatibility cost |
| S55-D1 | Policy G0–G3 and scope | G1, same Fang only, all Fang kinds through the shared acceptor |
| S55-D2 | Idle eligibility | Existing 60 s payload semantics; established secure/plain sessions, no setup/recent/sibling victim |
| S55-D3 | Resource/reclamation bounds | Existing caps; one shared victim; bounded nonblocking operation, real cleanup, at most 1 s and one reacquisition |
| S55-D4 | Deadline and provider interaction | Charge the original budget, retain provider preference and post-ACK commitment, no second victim |
| S55-D5 | Validation scope | Disposable native caps and deterministic race tests; secure-path reproduction before changing runtime |
| S55-D6 | Authorise M2/M3 after review? | Only after review of this proposal and its compatibility costs |

- **M1, this step:** production-cap plain-TCP reproduction, source evidence,
  alternative policies and retained validation; no runtime change.
- **M2 after review:** first reproduce secure global pressure without
  violating receiver/peer budgets, then implement the selected contract
  with tests for actual release, setup/active protection, pressure vanishing,
  one-victim sharing, cancellation/busy operation and original deadlines.
- **M3:** native global-pressure controls across Fang kinds, active/recent
  and source/local/peer interactions, routed/provider pressure, concurrency,
  empty-Fang ownership, release recovery, FD/log/CPU bounds and cleanup.
  Retain Stage54 64/32/21, provider and broader regression, full workspace
  checks, debug/release auth proofs, resolver/address/resource/fairness
  controls, S2/S3/S4/S7 P0 and full local CI.
- **M4:** limitations/status and closure from reviewed evidence only.

Frozen rc.1/rc.2 artifacts, S3-P2/S2-P4 exclusions, Stage38 closure,
Tokio 1.52.3 and advisory exceptions stay unchanged. Per-user fairness,
cross-Fang stealing, release certification, WAN testing and comparative
liveness remain outside this proposal. No installed node, real Den/peer
or service unit is used; private fixture identities are removed.
