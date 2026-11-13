# Stage50 — dependency security remediation survey

**Status: proposal for owner review (2026-11-13). No dependency change
or trial implementation has begun.** The fresh dependency audit blocks
Stage49 delivery, so security remediation takes priority over the
previously considered Git-hook cleanup. Stage49's functional regression
is complete; PR #158 stays draft and its closure is pending.

## 1. Baseline and the new blocker

- Current `main` after #157 (`6ac6594`) and the tested Stage49 branch
  (`0f5afa7`) pin quinn-proto 0.11.18 and quinn-udp 0.5.14. The frozen
  `v1.0.0-rc.2` lockfile pins those versions too. The audit findings were
  not introduced by the Stage49 probe correction.
- Fresh Stage49 results: 460 Rust tests passed in the recovered output;
  the original Cargo exit status is unavailable after a server restart,
  and a separate doc-test invocation exited 0. Formatting, Clippy,
  release build, 93 refusal scenarios, 40 address scenarios, 18 retry
  scenarios, QUIC resource floods and all 12 P0 chaos runs passed.
- Full local CI `target/ci-local/20261113T133214Z`: **12/13 PASS; audit
  FAIL**, exit 1. [Stage40 §2](STAGE40_CI_COVERAGE_SPEC.md) requires the
  audit, and `ci-local.sh` returns nonzero for a failing check. It is
  not overridden. No advisory ignore entry was added.
- The refreshed RustSec database is at
  `4ef2df8e806a65e2f1ecc14f56f51249062b23d6`. The three relevant local
  files were byte-compared with that Git commit and preserved under
  `target/stage49-postfix/advisories/`. The database assignment commit
  is dated 2026-11-13; the advisories themselves state 2026-09-30.
  Earlier clean audits describe their earlier database snapshot.
- Canonical advisory pages were unavailable through the browser tool.
  The primary evidence here is the freshly fetched RustSec Git snapshot,
  not a claim that the website was successfully checked.

## 2. Findings and affected paths

| Advisory | Pinned → minimum patched | Upstream finding and local assessment |
|---|---|---|
| [RUSTSEC-2026-0523](https://github.com/RustSec/advisory-db/blob/4ef2df8e806a65e2f1ecc14f56f51249062b23d6/crates/quinn-proto/RUSTSEC-2026-0523.md) | quinn-proto 0.11.18 → 0.11.19 | A post-handshake peer can bypass the bidirectional stream limit using a MAX_STREAM_DATA frame, causing resource exhaustion. The receiver repeatedly calls `accept_bi` ([code](../crates/werewolfd/src/transport/quic.rs)); exact application exposure needs M1 analysis. Advertising a small limit is not proof of immunity to a limit-bypass bug. |
| [RUSTSEC-2026-0524](https://github.com/RustSec/advisory-db/blob/4ef2df8e806a65e2f1ecc14f56f51249062b23d6/crates/quinn-proto/RUSTSEC-2026-0524.md) | quinn-proto 0.11.18 → 0.11.19 | An oversized Retry token can panic a debug client or trigger an excessive allocation in release. Upstream identifies outgoing clients as affected, including an attacker who observes the first Initial and races a response. Werewolf makes outgoing QUIC connections on direct, routed, failover, discovery and sync paths ([code](../crates/werewolfd/src/quic_fang.rs)). Selected-peer TLS pinning must not be assumed to protect this earlier packet-processing path. |
| [RUSTSEC-2026-0538](https://github.com/RustSec/advisory-db/blob/4ef2df8e806a65e2f1ecc14f56f51249062b23d6/crates/quinn-udp/RUSTSEC-2026-0538.md) | quinn-udp 0.5.14 → 0.5.16 in the 0.5 line | A truncated datagram can leave some platforms spinning in their receive loop. **Upstream explicitly excludes Linux**, so this survey does not claim the certified Linux target has that spin bug. The lockfile audit still reports the version. |

The dependency versions and caller paths above are **CONFIRMED** by the
lockfiles, source and advisory records. An exploit against Werewolf was
not run. Precise receiver reachability and resource impact are for M1;
do not assume every post-handshake peer has already passed Pack/OPEN
authorization.

## 3. Recommended candidate and scope

Prepare an **isolated minimal trial** of quinn-proto **0.11.19** and
quinn-udp **0.5.16**, retaining quinn 0.11.12, tokio 1.52.3 and the
other pinned dependencies wherever the solver permits. The proposed
commands are precise package updates, not a general `cargo update`.
Inspect the complete lockfile diff and stop for review if more than the
necessary dependency graph changes.

These are the minimum patched versions stated in the fetched records,
not a claim that they are the latest releases. M1 must verify the
upstream fixes and resolve an actual candidate lockfile before any
promotion. No candidate lockfile has been generated yet.

**Known trade:** [Stage34](STAGE34_S3_P2_RECORD.md) already tested
quinn-proto 0.11.19 and observed the same routed-QUIC loss limitation as
0.11.18. Updating for security does not fix or recertify S3-P2. Stage34's
future-recertification requirements require a release later than
0.11.19 or an approved configuration change; this minimal trial claims
neither. S3-P2 and S2-P4 remain EXCLUDED, and Stage38 remains CLOSED.

Keep the rc.2 tag/archive frozen. Update current security documentation
to record the new findings; do not rewrite an old audit result, replace
a release artifact, or label a candidate build certified. A new release
and any WAN campaign remain separate decisions.

## 4. Proposed milestones

- **M1 — exposure review and isolated trial.** Verify the three records
  and their upstream fixes; map receiver/client parsing to application
  authorization; create a separate candidate branch from the tested
  Stage49 source. Resolve the two precise versions and inspect every
  lockfile change. Run the fresh audit before expensive validation.
- **M2 — validate the candidate.** Formatting, Clippy, full Rust suite,
  locked release build, full disposable local CI and Stage12 transport
  proofs; repeat Stage49 refused/silent/race controls, resource pressure
  and the required P0 regressions on the candidate. Preserve the
  existing timing, memory, stream, socket and authentication bounds.
- **M3 — owner disposition.** Report pass/fail results and all unresolved
  exposure or compatibility questions. Promote only the reviewed
  candidate after the required gates pass. Coordinate delivery with
  blocked Stage49 PR #158; do not merge the vulnerable graph merely to
  unblock the administrative sequence. No advisory-ignore workaround is
  proposed.
- **M4 — records.** Update the security/dependency records and complete
  Stage49 closure on the finally validated build. A successful security
  trial is not comparative transport certification or a new release.

No live nodes, real peer keys, installed services or WAN hosts are
needed. Tests use disposable fixtures and retain no generated keys.
GitHub billing and hosted-workflow changes are outside this scope.

## 5. Other priorities

The tracked Git-hook installer still runs legacy live-node operations
before safe local CI. Isolating that workflow remains a bounded follow-up
([installer](../scripts/install-git-hooks.sh),
[Stage40](STAGE40_CI_COVERAGE_SPEC.md)), but it does not repair the audit.
QUIC DNS freshness, half-close semantics, the routed-loss diagnosis and
per-Fang remote-quota sharing keep their existing records and need
separate designs.

## 6. Owner decisions

| ID | Decision | Recommended |
|---|---|---|
| S50-D0 | Next engineering priority | Address the new dependency advisories before Stage49 merge; defer Git-hook cleanup. |
| S50-D1 | Candidate | Isolated minimal trial: quinn-proto 0.11.19 and quinn-udp 0.5.16; inspect all transitive changes. |
| S50-D2 | Release and verdict boundaries | Frozen rc.2 remains unchanged; S3-P2/S2-P4 stay EXCLUDED; no Stage38 reopening, tokio promotion or new release. |
| S50-D3 | Implementation authorisation | Approve M1–M4 as a new dependency-remediation task, with promotion only after reviewed passing evidence. |
