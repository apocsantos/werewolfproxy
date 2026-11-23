# Stage55 M3 — full-cap regression

**Status: M3 owner-merged in PR #176; M4 closure is for review.** M2 PR #175 merged as
`16d67a572a471f090f2e95ab0cca78588e92b42b`. M3 adds native controls at the
real 1024 global-session ceiling and reruns the Stage54 regression and local
gate scope. No runtime policy or cap change is part of this step.

## Full-cap results

The final supplemental campaign is pinned to fixture commit
`f885ffdfb074023bc8898c584126272170b6d330`. Its checkpoint passed all five
full-cap modes and all 13 local-CI gates on 2026-11-23:

| Mode | Cases | Result |
| --- | ---: | --- |
| Direct global admission | 9 | Pass |
| Routed secure owner | 2 | Pass |
| Provider fallback owner | 2 | Pass |
| Source-bound owner | 1 | Pass |
| Concurrent probes | 3 | Pass |
| Local CI | 13 gates | Pass |

Each direct case held 32 sessions on its owner and 992 on independent
plain-TCP siblings for the real 65-second idle interval. Across the nine cases
the fixture recorded 27 owner victims, preserved all 8,928 sibling sessions,
and completed 9,216 survivor echoes. The routed cases held the secure owner
through a generated relay. The provider cases intentionally denied the
configured primary target so the fallback peer path was exercised at the full
global cap; the broader Stage54 provider matrix separately covers active
primary, commitment and post-ACK behavior below that cap.

The source-bound case used TCP in a private loopback network namespace, with
the 32 secure owner sessions split across two source addresses. Its three
probes produced two actual victims and one successful admission; the released
slot from a failed secure setup was refilled before the full-cap survivor
checks. The concurrent cases exercise twelve simultaneous probes over three
transports. The fixture counts actual reset/EOF victims and records any refill
needed after a setup failure, rather than assuming every successful echo maps
one-to-one to a victim.

Every case checks actual echoes, owner-victim counts, sibling and empty Fang
protection, recent-session protection, the local-64 positive control,
release-one recovery, receiver authority, daemon survival, idle CPU, descriptor
drain and owned-process cleanup. Native controls use generated local nodes and
do not lower the global limit or receiver quotas.

## Retained regression scope

The broader campaign at `target/stage55-m3-final/` passed 513 Rust tests,
formatting, lint and audit checks, release build, authentication proofs, the
Stage54 provider/share/boundary/route/source matrix, Stage53 membership/path
checks, Stage52 resolver/migration checks, address/refusal/resource/fairness
controls, four P0 chaos cells and all 13 local-CI gates. The final supplemental
evidence and hashes are in `target/stage55-m3-close/`; both directories are
ignored campaign evidence, not committed artifacts. Earlier fixture
development failures are preserved in `target/stage55-m3/`.

The final checkpoint also verified that runtime sources, manifests, lockfile,
tests, scripts, release inputs and workflows match merged M2. M3 changes only
the Stage55 regression fixture and its documentation.

## Boundaries

These are functional loopback regressions, not throughput or comparative
liveness measurements. The source-bound full-cap case is one TCP case in a
private loopback namespace; it does not establish WAN behavior or universal
admission under races. The provider full-cap case exercises fallback when the
primary target is unauthorized; active-primary and commitment scenarios remain
covered by the Stage54 matrix below the global cap. The concurrency fixture
measures setup failures and refills free slots, and makes no cross-Fang fairness
claim. Frozen release artifacts, dependency versions, advisory exceptions and
operator settings remain unchanged.
