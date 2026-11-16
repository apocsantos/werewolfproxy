# Stage50 — dependency security remediation closure

**Status: CLOSED on `main`, 2026-11-16.** The owner merged
[PR #159](https://github.com/apocsantos/werewolfproxy/pull/159) as
`c1e78459b7528ac1f30d164eb7eec5230ee9aa9b` at 15:09:02 UTC.
This completes the remediation and resolves the Stage49 dependency-audit
blocker on `main`. Frozen rc.1/rc.2 releases remain unchanged.

## Changes and provenance

- quinn-proto **0.11.18 → 0.11.19** addresses RUSTSEC-2026-0523/-0524.
- quinn-udp **0.5.14 → 0.5.16** addresses RUSTSEC-2026-0538; its upstream
  advisory explicitly excludes Linux.
- Workspace and standalone Stage12 Quinn-proof lockfiles agree. Proof
  version labels match. The obsolete Windows dependency subtree was
  removed; unrelated Windows selections were preserved.
- Quinn remains 0.11.12, Tokio 1.52.3 and Rustls 0.23.46. Application
  source, transport/resource bounds and advisory ignores did not change.

Tested candidate: `7ed3a91fd594f4ea49b7e336d151d076c214e0fc`.
Published head `9d0a517c9a8a4faf376201dc256f6f444bf88a34` adds documentation
only; merged validation inputs match that candidate. The original
[survey](STAGE50_DEPENDENCY_SECURITY_SURVEY.md) and
[trial](STAGE50_DEPENDENCY_TRIAL.md) preserve their historical scope.

## Validation and delivery

All 19 required checks recorded actual exit 0. Results include:

| Check | Result |
|---|---|
| Workspace and Quinn-proof audits | PASS, final RustSec snapshot `02e2517f457a359fc6d3440ea4560c8ea188fee8` |
| Formatting, Clippy, locked release build | PASS |
| Rust suite | 460 passed |
| Stage12 TLS / Quinn proofs, debug and release | 8/8 and 3/3 respectively |
| Stage49 refusal / address controls | 31/31 and 10/10 |
| Descriptor/quota controls | 256-fd gate and three 64-fd repeats PASS |
| Stage32 S2/S3/S4/S7 P0 | 12/12 |
| Full disposable local CI | 13/13 PASS |

Evidence is indexed in `target/stage50-trial/verified.json` and the trial
record; full CI is `target/ci-local/20261116T123910Z/summary.tsv`.
Safe prepublication fast checks also passed at
`target/ci-local/20261116T150404Z`. Publication bypassed the documented
legacy pre-push hook because it invokes installed-node administration
and persistent target servers. No installed node or real peer was used.

Hosted CI supplies no passing result: all seven jobs on the published
head were prevented from starting. Each annotation reports:
"The job was not started because your account is locked due to a billing issue."
Runs: [Rust CI](https://github.com/apocsantos/werewolfproxy/actions/runs/43892494434)
and [CodeQL](https://github.com/apocsantos/werewolfproxy/actions/runs/43892588721).
No billing or workflow setting was changed.

Stage49's original 12/13 audit failure and its recovered Rust run's
missing exit status remain recorded in [M3](STAGE49_M3_REGRESSION.md).
They are not converted into passes; this later candidate supplies the
passing audit and actual Rust exit status.

## Remaining limits and next task

One original blackholed-provider client took **7.44 s**. Three fresh
targeted repeats (nine clients) echoed in **5.03–5.07 s**. The original
observation remains; its cause is unproven and no bound was relaxed.

This is functional local validation, not comparative liveness or release
certification. S3-P2/S2-P4 stay EXCLUDED and Stage38 stays CLOSED. The
routed-loss limitation remains. No Windows runtime or WAN testing,
Tokio promotion, new release or frozen-artifact replacement occurred.

The bounded follow-up recommended by the original survey is Git-hook
cleanup: separate routine local push checks from the installer's legacy
live-node operations. It remains a separate engineering task.
