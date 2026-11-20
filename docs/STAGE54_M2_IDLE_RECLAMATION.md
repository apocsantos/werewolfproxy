# Stage54 M2 — bounded idle reclamation at peer shares

**Status: M2 merged in [PR #171](https://github.com/apocsantos/werewolfproxy/pull/171), 2026-11-20.**
Merge commit: `a8bcc1266a8c68808cccfcc0cbca6ba0d27ac13a`.
The [M3 regression record](STAGE54_M3_REGRESSION.md) tracks continuation;
the validation below retains the reviewed M2 candidate's evidence.
The owner merged [PR #170](https://github.com/apocsantos/werewolfproxy/pull/170)
as `4dd8ddaf735084041c8e6f34a2566383138f8494` and instructed continuation.
The [reviewed design](STAGE54_IDLE_PEER_SHARE_SURVEY.md) decisions
S54-D0–D6 are applied as recommended: I2, including current-provider idle
reclamation before fallback. This record covers M2, not Stage54 closure.

Fixed runtime/test/fixture commit:
`52130afb46c0a42765740df6220d59afb8835616`, following initial candidate
`705aba1391bdd3910e3d2bb7854370bcfe3d0206`.

## Resulting behaviour

A local peer-session capacity denial can replace one eligible idle
session from the same Fang. The victim must be committed to its final
destination, have no observed successful payload I/O for at least the
existing **60 s**, and actually hold the receiver keys that constrain
the new attempt. Removing just that reservation must make every demanded
share/total check pass. Longest idle wins; equal timestamps use the
smaller stable session identifier.

Setup work, recent activity, unrelated/sibling Fangs and already-evicting
sessions are excluded. A grandfathered Fang cannot evict several clients
to force itself below a smaller share. No useful single victim means the
ordinary denial/fallback result. This may reset a legitimate idle SSH or
database connection below the former 64-slot local ceiling, as reviewed.

Every newcomer has one victim budget shared across local, source and
peer-pressure triggers. One nonblocking semaphore serializes peer
reclamation within each Fang. Other newcomers do not queue behind it;
they follow ordinary denial/fallback handling. Marking a victim does not
credit capacity. Wait at most the existing **1 s**, bounded also by the
remaining attempt/selection deadline, then acquire the whole demand set
once after real cleanup. Races, slow cleanup and remote lease lag can
still deny admission.

The wait is charged to the first dial's existing **5 s** window, including
hostname refresh/capacity wait and address racing. It does not create a
fresh five-second dial window afterward. An existing Stage32 TCP setup
retry keeps its later retry window. Failover's existing **15 s** selection
deadline includes reclamation; provider changes do not restart it.

Direct QUIC/TCP, routed and failover Fangs use the same coordinator and
actual receiver-key demand sets. A routed reservation remains atomic for
relay and destination. At each provider, eligible own-Fang idle capacity
is reclaimed before moving to a fallback. Active/recent primary capacity
denial still uses fallback in existing order. Once the destination ACK
verifies, payload failure resets that client without provider replay.

## Lifetime, locking and commitment

[Session tracking](../crates/werewolfd/src/transport/fang_accept/session.rs)
connects the existing Fang activity/cancellation registry to each live
reservation. Metadata stores its actual attempt, not the union of unused
provider alternatives. A short task-local context identifies the owned
Fang slot; payload tracking itself retains the existing Stage43 paths.

Commit markers occur after direct QUIC ACK, direct TCP ACK, the routed
destination's inner ACK, or the selected direct provider's ACK. The
relay's outer ACK and generic framed-copy pump do not mark a routed
session committed. Prepared and failed provider attempts remain ineligible.

Lock order is **Fang registry → peer coordinator**. Ordinary reservation
releases the coordinator lock before attaching metadata. Reclamation
inspects actual usage under the same order. Reservation destruction
removes candidate metadata while holding the registry, releases real
coordinator usage, then notifies the waiter. It cannot expose a stale
candidate whose reservation was already released. No mutex spans an
await, dial or cancellation wait. Cancelling a waiter drops its gate;
the victim's capacity stays counted until owned cleanup actually ends.

Only the typed local capacity-denial error triggers reclamation.
Unavailable/invalid coordinator state, resolver/remote/handshake failures
and post-ACK errors do not. Ordinary local denials remain `dial`; health
aggregation and the ten-second logging limit remain. Peer evictions use
their own existing-style bounded event log.

No dependency, wire, persistent-state or CLI format changes. Receiver
authority remains final. The 64-slot local/per-peer, 1,024 daemon-local
and global receiver, 64-member and per-source bounds stay unchanged.
Plain TCP retains its existing local/source idle rules.

## Focused tests and disposable controls

All **12 new Rust tests pass** in the final workspace suite. The initial
eleven focused tests cover useful
committed replacement, first-dial budget charging, setup/recent-byte
protection, actual provider keys, atomic routed constraints, over-share
grandfathering, expired/invalid attempts, longest-idle tie order, slow
cleanup with no speculative capacity, one-victim reuse across providers,
busy gate/cancelled waiter cleanup, local/source shared victim budgets,
distinct memberships and membership growth/shrink during the wait.
Unit policies use short thresholds; production remains 60 s and the
disposable controls use real 65-second idle intervals.

Review added a twelfth regression after the initial candidate completed
all 16 checks. Another session can release requested-key capacity between
the initial denial and candidate selection. Checking only post-release
capacity could then select an unnecessary idle victim, including one at
an unrelated provider. The added test failed on `705aba1` with actual
exit 101, then passed unchanged with the correction. Selection now also
requires at least one demanded key to remain blocked at its checked
snapshot. If pressure has disappeared, no victim is useful; ordinary
denial/fallback handling may still win that race. This does not add a
reservation retry loop or speculative capacity.

The pre-fix failure and unchanged-test hashes are retained under
`target/stage54-m2-race/`. Initial successful campaign evidence remains
under `target/stage54-m2/`; it is not substituted for final-source results.

The [idle fixture](../tools/stage54/README.md) retains its default baseline
expectations and adds `--expect reclaimed`. Three QUIC/TCP rounds test
64/32/21 shares. Before the idle interval, excess clients still reset.
Afterward, every newcomer must echo and replace exactly one old idle
client, including three successive newcomers at each smaller share.
Active siblings and independent peers stay usable; release-one recovery,
authority, descriptor baseline, target-handler drain and private cleanup
remain checked.

The new provider fixture covers direct and routed QUIC/TCP paths. At
32 recent primary sessions it must use fallback, then after 65 s of
primary inactivity admit three newcomers at the primary with exactly
three old resets and no new fallback payload streams. All 32 remaining
committed primary sessions must reset on a target fault without replay;
the already-established fallback remains usable. The primary must recover
without reopen. Independent held/new clients and complete resource/private
cleanup remain checked. No installed node, real peer or WAN host is used.

## Final-source validation

**PASS on fixed source `52130af`.** All **16 campaign checks** recorded
actual worker exit 0 and passed scenario verifiers, including the final
497-test suite and the complete real-idle share/provider controls. All
**316** source-input hashes, plus the driver/helper hashes, still match.
Existing manifests, lockfiles, test assertions/gate definitions, workflows
and release inputs match merged M1 `4dd8dda`.

| Required control | Final-source result |
|---|---|
| Workspace / standalone Quinn-proof audits | PASS; advisory policy unchanged |
| Formatting, all-target/all-feature Clippy, locked release build | PASS |
| Locked offline workspace/all-feature Rust suite | **497 passed**, including all 12 new safety tests |
| TLS / QUIC proofs, debug and release | **8/8** and **3/3** each; UDP environment PASS |
| Real-idle 64/32/21 shares, three QUIC/TCP rounds | **18/18 groups** in six fresh labs |
| Idle direct/routed provider controls | **4/4 paths** |
| Existing two-Fang shares / provider and activation controls | **12/12** and **5/5** |
| Stage52 hostname migration without reopening | **21/21** |
| Full disposable local CI | **13/13 PASS**, all actual gate exits 0 |

The six share runs admit **42** post-idle newcomers with exactly **42**
old idle resets. All **12** release-one recoveries and **18** new
independent-peer clients echo. Each real idle interval is at least 65 s.
The four provider paths admit **12** newcomers through reclaimed primary
capacity, reset exactly **12** older idle clients, then produce **128**
committed resets on target faults with **zero** additional fallback
payload streams. All four active-primary fallback controls, existing
fallback sessions, primary recoveries and **12** new independent-peer
clients pass. These counters measure payload-bearing streams, not every
empty handshake.

Every new share/provider lab returns descriptors **16 → 16**, all receiver
active/establishing authority to zero, and drains target handlers before
owned-process/private-directory cleanup. No log guard fired. The source
keeps the 60 s eligibility threshold and 1 s release bound; no timing or
resource criterion was weakened.

Evidence root: `target/stage54-m2-recheck/`, including `verified.json`,
`validation-meta.json`, `summary.tsv`, per-worker command/state/result
records, raw scenario JSON and separate scenario summaries. Every share,
provider, two-Fang, activation and migration row uses lab daemon SHA-256
`6183b3155cb5613b8be193dbbebac2a7a425f713477748b72ddffbe86870f43b`.
CI/proofs retain their separate build-path provenance.
Lockfile SHA-256 remains
`411ef3b496dfd8c21f97eb2285d55bd0d12ae604436acfdfa16044e80a0d91d0`.

Full local CI is `target/ci-local/20261119T230511Z/summary.tsv`, including all
**68 Python unit tests** and audit. All four documents remained unchanged
through the campaign and full CI; this final results note followed
completion. Python syntax/CLI help, all **86** local document links checked
and whitespace checks pass. RustSec snapshot observed after the final
suite: `074ea9a7ed619f511b5b7374c7a97e98d1e98be4`.

The successful initial `705aba1` campaign and expected red/green
regression remain in their separate evidence roots above. They are not
substituted for this final-source run. M2's added regression strengthens
the useful-victim condition; the baseline fixture expectations and all
existing test assertions/gates remain available unchanged. These ignored
local records are not committed release artifacts or private identities.

## Remaining work and boundaries

M3 remains the broader regression required by the reviewed design:
22-slot remainder, mixed paths, recent activity, non-loopback sources,
half-close, over-share and membership changes, common relay/destination
constraints, concurrent pressure/slow cleanup, resolver faults,
address/refusal/resource controls and S2/S3/S4/S7 P0. M4 limitations and
closure follow that reviewed evidence. M2 does not claim universal
fairness, admission after every attempted reclamation, per-user shares
or service during active saturation.

Frozen rc.1/rc.2 tags and archives, S3-P2/S2-P4 exclusions, Stage38 closure,
Tokio 1.52.3 and advisory exceptions remain unchanged. These are local
functional controls, not comparative liveness, throughput or new release
certification. Generated private identities are removed.
