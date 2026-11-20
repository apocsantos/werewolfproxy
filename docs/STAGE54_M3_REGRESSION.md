# Stage54 M3 — idle-reclamation regression

**Status: campaign in progress.** The owner merged
[PR #171](https://github.com/apocsantos/werewolfproxy/pull/171) as
`a8bcc1266a8c68808cccfcc0cbca6ba0d27ac13a` on 2026-11-20 and instructed
continuation. M3 preserves that runtime, dependencies, existing tests,
gate definitions, workflows and release inputs. The
[reviewed design](STAGE54_IDLE_PEER_SHARE_SURVEY.md) and
[M2 implementation](STAGE54_M2_IDLE_RECLAMATION.md) define the contract.
Stage54 closure follows reviewed M3 results in M4.

## New disposable coverage

[Fixture and invocation](../tools/stage54/README.md). Every native idle
observation lasts at least 65 monotonic seconds, beyond the unchanged
60-second threshold. All clients first echo, targets have no application
idle timeout, and successful held clients remain open during observation.

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

## Scope

These are local functional checks, not comparative liveness, throughput,
WAN or release certification. The one-victim rule permits denial after
an unsuccessful reclamation race; it does not guarantee every newcomer
admission. There is no borrowing, forced multi-victim rebalance, per-user
fairness or service guarantee during active saturation.

Frozen rc.1/rc.2 artifacts, S3-P2/S2-P4 exclusions, Stage38 closure,
Tokio 1.52.3 and advisory exceptions remain unchanged. No live peer,
installed node or service unit is used. Generated identities are removed.
