# Stage54 M3 — idle-reclamation regression

**Status: M3 complete and locally validated; for review, 2026-11-21.** The owner merged
[PR #171](https://github.com/apocsantos/werewolfproxy/pull/171) as
`a8bcc1266a8c68808cccfcc0cbca6ba0d27ac13a` on 2026-11-20 and instructed
continuation. M3 preserves that runtime, dependencies, existing tests,
gate definitions, workflows and release inputs. The
[reviewed design](STAGE54_IDLE_PEER_SHARE_SURVEY.md) and
[M2 implementation](STAGE54_M2_IDLE_RECLAMATION.md) define the contract.
Stage54 closure follows reviewed M3 results in M4.

Fixed fixture and campaign source commit:
`4b48bb5b3ff2473684ce9e9c2875bd97560c1028`.
All **31** campaign workers and the overall manager exited **0**. The
campaign finished at **2026-11-21 00:29:57 UTC**. Result notes were added
after full local CI; runtime and fixture source bytes remain unchanged.

## New disposable coverage

[Fixture and invocation](../tools/stage54/README.md). Every native idle
observation lasts at least 65 monotonic seconds, beyond the unchanged
60-second threshold. All clients first echo and targets have no application
idle timeout. Retained sessions stay open during observation; the
transport-specific target-first half-close expectations are listed below.

| Cases | Required observations |
|---|---|
| Boundary, QUIC and encrypted TCP | First-registered 22-slot remainder with mixed sibling transports; three idle replacements; two continuously active and one newly refreshed owner session protected; active siblings untouched |
| Late activation in boundary cases | Old Fang retains 63 idle sessions after its share falls to 32; one free total slot admits the late Fang; removing one old session could not fit the old Fang, so its newcomer must deny without forced eviction; closing old Fang restores late Fang's 64 share |
| Provider budget in boundary cases | Main Fang owns 22 sessions at each provider, below its local 64 ceiling; one peer victim at primary, then authenticated target refusal; full idle fallback cannot consume another victim |
| Half-close in boundary cases | Target finishes downstream after echo while retaining upload; QUIC retains 32 reservations and pressure reclaims exactly one; encrypted TCP's target-first completion drains reservations promptly |
| Common relay and common destination, both hop transports | Two captured paths own 32 sessions each; three sequential own-Fang replacements; twelve concurrent newcomers at concurrency six admit only within the one-victim bounds; sibling's 32 sessions survive |
| Active routed storms | Refresh all live clients, then 600 newcomers at concurrency 16 must all reset locally as `dial`, with exactly 600 health failures, zero target payload growth and bounded denial logs |
| Non-loopback sources, both transports | Four members reserve 16 slots each; source-limit eviction and peer-only eviction each replace one idle session; active siblings and refreshed clients survive |
| Source-first provider budget | Main owns 21 primary and 22 fallback sessions distributed across three source addresses; one spare primary peer slot isolates target refusal from source-cleanup timing; source-limit victim at primary precedes refusal, and full idle fallback must not consume a second victim |

Source cases re-execute this fixture in a private user, mount and network
namespace with a private root and 512 MiB temporary filesystem. Only
`10.54.0.1–4/32` on its own loopback interface are configured. These
non-loopback source addresses exercise source accounting locally; they
do not represent WAN traffic or changes to host networking.

Each native lab checks independent held/new admissions, daemon survival,
zero final active/establishing receiver authority, drained target handlers,
initiator descriptors at or below the pre-Fang baseline and two seconds
of idle initiator CPU below 10%. Existing 50 MiB per-log guards apply.
Only owned processes are stopped and generated private Dens are removed.
Outputs must be new; source hashes, phases and rows are checkpointed, and
any unexpected outcome stops the run with its evidence retained.

Source admission can cancel one idle client while local Fang slots remain
free, then reach peer reservation before the cancelled session's cleanup
releases its quota. A reset of that newcomer is permitted: its victim
budget is already spent. The source fixture checks exactly one old victim,
real receiver release, and recovery in the resulting free slot without a
second victim. It then fills the share and tests a separate peer-only
replacement. Half-close reclamation is measured by original target-handler
identities and receiver authority; a client that already consumed FIN
continues to report EOF when its retained upload is cancelled.

## Regression procedure and evidence

The final campaign repeats the Stage54 64/32/21 matrix with reclamation
expectations and all four direct/routed provider controls, plus three
rounds of each new case. Existing assertions remain unchanged in the
Stage53 shares/membership/provider, Stage52 resolver/migration,
Stage46/49 address/refusal and Stage42/43 pressure/fairness tools.
S2/S3/S4/S7 P0 controls run three times per cell.

The unchanged Rust suite supplies deterministic unit coverage for
membership growth/shrink during the release wait, slow cleanup, actual
receiver-key selection, concurrent/busy reclamation, cancellation, the
shared victim budget, deadline charging and pressure disappearing before
selection. Those tests use unit-level timing controls; native labs do
not claim to inject slow cleanup or membership changes at an exact await.

Workspace format, Clippy, all-feature tests, release build, debug/release
TLS and Quinn authentication proofs, advisory audits and full local CI
complete the campaign. Each worker records its actual process exit,
source commit, PID identity, duration and logs. Scenario JSON is verified
separately from worker exit codes. Full local CI runs with the tracked
and unignored tree fixed; final result notes follow completion.

## Verified results

Every row below passed its scenario verifier as well as its actual worker
exit check. Earlier M2 evidence and development smokes were not used as
substitutes for these fresh runs.

| Coverage | Final campaign result |
|---|---|
| Workspace and audits | Format, Clippy with warnings denied, all **497** Rust tests, release build, workspace and Quinn-proof advisory audits pass |
| Authentication proofs | Debug and release each pass TLS **8/8** and Quinn **3/3**, including UDP environment and pinned Quinn versions |
| Stage54 64/32/21 | **18 groups / six labs**, three rounds per transport; **42** admitted idle replacements and exactly **42** old resets; all twelve smaller-share release-one recoveries and eighteen independent new clients pass |
| Idle providers | All **four** direct/routed paths; **12** idle-primary admissions with twelve old resets, active fallback preserved; **128** committed primary resets with **zero** fallback payload replays; primary recovers without reopen |
| New boundary matrix | All **six** labs; 22-slot mixed-transport remainder, active/recent protection, 63-session grandfathering and shrink recovery; one peer victim and no second-provider victim per budget probe |
| Half-close | All three QUIC labs retain 32 uploads and release exactly one original target handler under pressure; all three encrypted-TCP labs promptly drain target-first sessions |
| New routed matrix | All **twelve** labs; three sequential replacements each, plus **85** concurrent admissions and exactly **85** concurrent victims across batches of twelve; all 32 sibling sessions survive in each lab |
| Routed active storms | **7,200 / 7,200** newcomers reset as local `dial`, exactly 7,200 health failures, **zero** new target payload streams; no additional denial log lines during the storm intervals |
| New source matrix | All **six** labs; one source-limit victim, one separate peer-only victim and one source-first provider-budget victim each; no second-provider victim; all six initial source probes reset in the permitted cleanup race, followed by six free-slot recoveries without another victim |
| Stage53 controls | **12** share cases with both arrival phases, **39** membership cases and **15** provider/activation cases across three rounds |
| Stage52 controls | **39** resolver-fault and **21** migration cases across three rounds |
| Address/refusal controls | **40** address and **31** refusal/datagram scenarios; prompt QUIC-refusal bounds pass; 10,000 datagrams produce no receiver log lines and all six recovery echoes pass |
| Stage42 pressure | **12** resource scenarios: six at descriptor limit 256, plus QUIC/TCP at limit 64 in three independent repeats; recovery, bounds, listener survival and descriptor drain pass |
| Stage43 fairness | One full **six-case** pass: idle 65 s and active-source 30 s over QUIC, encrypted TCP and plain TCP; original assertions retained |
| P0 chaos | S2/S3/S4/S7 each pass **three trials**, twelve total; S7 is the secure-selector control |
| Full local CI | **13/13** actual gate exits zero, including all **68** Python unit tests and the advisory audit |

All **34** Stage54 native labs (six share, four provider and 24 new labs)
use full 65-second observations, survive, return initiator descriptors
**16 → 16**, drain target handlers, end with zero active/establishing
authority at every checked receiver and remove all owned processes/private
Dens. New boundary/routed/source labs measure idle initiator CPU at
**0–0.49%** over two seconds. Concurrent routed admissions range from
**3 to 10** per twelve-client batch. These observations do not promise
admission for every racing newcomer.

### Provenance and retained failures

Evidence root: `target/stage54-m3/`. `summary.tsv` and per-worker
spec/running/result records preserve all 31 actual exits and durations.
`verified.json` checks the complete scenario results;
`results-summary.json` aggregates them and records matrix digests.
`manager-result.json` records observation of the overall native exit 0.
These ignored local files are evidence, not committed release artifacts
or private identities.

Full local CI: `target/ci-local/20261121T002416Z/`, **340.8 s**.
P0 evidence: `target/stage32-chaos/results/20261121T001640Z-network/`.
The chaos certification and fault-injection builds retain their own
feature/hash provenance; they are separate from the native debug lab.

All **317** tracked source-input hashes match the fixed campaign source.
The reused helper and campaign driver hashes are retained with the
checkpoint metadata. Native debug daemon SHA-256:
`6183b3155cb5613b8be193dbbebac2a7a425f713477748b72ddffbe86870f43b`.
Lockfile SHA-256:
`411ef3b496dfd8c21f97eb2285d55bd0d12ae604436acfdfa16044e80a0d91d0`.
RustSec snapshot observed after the final suite:
`f291af9b1cc0d21dfcea1709d6b037963cef262b`.

Separate fixture-development evidence remains under
`target/stage54-m3-smoke/`, indexed with digests and actual exits.
The first four invocations exited **1**: incorrect provider membership
setup, expecting a new RST after already-consumed FIN, assuming admission
before source cleanup released quota, and observing TCP cancellation
before its asynchronous client reset arrived. Every failed lab cleaned
up successfully. The fifth source-TCP smoke exited **0**. Corrections
isolated the intended memberships and measured real handler/quota release
and bounded client outcomes. The final frozen campaign then passed all
eight new cases in three fresh rounds, with no failed campaign worker,
silent rerun, shortened idle threshold or runtime/assertion change to the
existing gates.

Final source/protected-runtime checks, all **93** local document links
across the Stage54 records/index/README, and whitespace checks pass.
Only documentation result notes differ from the fixed campaign commit.

## Scope

These are local functional checks, not comparative liveness, throughput,
WAN or release certification. The one-victim rule permits denial after
an unsuccessful reclamation race; it does not guarantee every newcomer
admission. There is no borrowing, forced multi-victim rebalance, per-user
fairness or service guarantee during active saturation.

Frozen rc.1/rc.2 artifacts, S3-P2/S2-P4 exclusions, Stage38 closure,
Tokio 1.52.3 and advisory exceptions remain unchanged. No live peer,
installed node or service unit is used. Generated identities are removed.
