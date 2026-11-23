# Stage55 — global idle reclamation closure

**Status: M1–M3 owner-merged; M4 documentation closure for review, 2026-11-23.**
This record summarizes the delivered policy, regression evidence and remaining
limits. M4 changes documentation only; it does not claim new runtime or test
results.

## Delivery

| Milestone | Owner-merged delivery | Record |
| --- | --- | --- |
| M1 baseline and policy proposal | [PR #174](https://github.com/apocsantos/werewolfproxy/pull/174), `5ce2a54` | [Survey](STAGE55_GLOBAL_IDLE_ADMISSION_SURVEY.md) |
| M2 bounded global idle reclamation | [PR #175](https://github.com/apocsantos/werewolfproxy/pull/175), `16d67a5` | [Implementation](STAGE55_M2_GLOBAL_RECLAMATION.md) |
| M3 full-cap and retained regression | [PR #176](https://github.com/apocsantos/werewolfproxy/pull/176), `80c86a5` | [Results](STAGE55_M3_REGRESSION.md) |
| M4 limitations and closure | this PR | this record; documentation only |

The measured boundary was a Fang with local room being denied at the daemon's
1024-session ceiling while it held established idle sessions of its own. M2
implements the reviewed G1 policy in the shared Fang acceptor. At actual global
exhaustion, one useful established session in that same Fang, idle for at least
60 seconds since successful payload activity, may be cancelled. Setup, recent,
already-evicting and sibling sessions are excluded. The candidate must make
the current admission useful, and current pressure/activity are rechecked
before marking.

Cancellation does not itself free capacity. The owner cleanup path must release
the actual local/global permits. One nonblocking reclamation operation per
Fang waits at most one second within the original setup/dial/selection budget,
then attempts global acquisition once. The newcomer carries one shared victim
budget across local, source, global and peer-share reclamation and provider
attempts. There is no global semaphore queue, unbounded retry or second victim.
Provider order, receiver authority, captured route/identity demands and final
destination ACK commitment remain intact.

## Reviewed validation

M2 deterministic and full-cap validation is recorded in the M2 report. The M3
campaign used fixed fixture commit `f885ffdfb074023bc8898c584126272170b6d330`
and passed:

| Coverage | Result |
| --- | --- |
| Direct full-cap admission | 9 cases; 27 owner victims, 8,928 sibling sessions preserved, 9,216 survivor echoes |
| Routed secure owner | 2 full-cap cases passed |
| Provider fallback owner | 2 full-cap cases passed with primary target unauthorized |
| Source-bound owner | 1 TCP full-cap case passed in a private loopback namespace |
| Concurrent pressure | 3 full-cap cases passed over three transports |
| Local CI | All 13 gates passed; pre-push fast CI passed |
| Retained broader regression | 513 Rust tests, formatting, Clippy, audits, release/authentication checks, Stage52–54 controls and four P0 chaos cells passed |
| Hosted checks on PR #176 | Tests, release build, formatting, Clippy, CodeQL and Actions/Python/Rust analysis passed |

The M3 checkpoint records exact worker exits and hashes under the ignored
`target/stage55-m3-close/`; the broader campaign evidence is under ignored
`target/stage55-m3-final/`. These are retained local evidence, not committed
release artifacts. The final checkpoint verified runtime sources, manifests,
lockfile, tests, scripts, workflows and release inputs against the merged M2
baseline; M3 changed the regression fixture and documentation only.

## Remaining limits

- A legitimate idle application session may reset below its local 64-slot or
  receiver-share ceilings when the Fang needs global capacity. Payload idleness
  does not prove application-level idleness.
- Reclaiming one victim does not guarantee admission. Slow cleanup or a
  competing Fang may take the released global slot first; the newcomer then
  denies without selecting another victim.
- An empty Fang has no own victim and remains denied when siblings occupy the
  global cap. Active sessions are never preempted. The policy does not provide
  cross-Fang or per-user fairness, borrowing, rebalance, or protection from
  active floods and remote saturation.
- Full-cap source-bound coverage is one TCP case in a private loopback
  namespace. Full-cap provider coverage exercises fallback when the primary
  target is unauthorized; Stage54 covers active-primary and commitment paths
  below the global cap. No WAN, throughput, comparative-liveness or universal
  admission claim follows.

Frozen rc.1/rc.2 artifacts, dependency versions, advisory exceptions, operator
settings, wire formats and persistent formats remain unchanged. See
[known limitations](../KNOWN_LIMITATIONS.md) and [current status](STATUS.md).
