# Stage40 — CI coverage spec (for owner review)

**Status: DRAFT, documentation only.** Nothing in this spec is
implemented until the owner approves the decisions in §6.

Source: Stage39 survey §11 (testing gaps) and §13.3 (recommendation 3).

## 1. Problem

`.github/workflows/rust.yml` runs only `cargo fmt`, `clippy`,
`cargo test` and a release build. These checks run only at stage gates,
by hand, on the owner's host:

| Check | Size | Needs | Runs in CI today |
|---|---|---|---|
| Python unit tests (`tests/integration/test_*.py`) | 64 tests, ~2 s, stdlib only | Python 3.11+ | no |
| Acceptance (`tests/integration/run.py`) | 12 assertions | Rust build, loopback TCP/UDP, Unix sockets | no |
| Ten lab gates (`tests/integration/*_gate.py`) | activation, auto_sync, death, discovery, governance, provider, routed_fang, service, service_discovery, sync | as acceptance | no |
| Advisory audit (`cargo audit`) | — | `cargo-audit` | no |
| Stage32 chaos campaigns (`chaos.py`) | hours | `unshare -rmnp`, netem, faketime | no, **and out of scope** |

A regression in the authorization matrices, transport policy or the lab
gates is therefore caught only at the next stage gate.

## 2. Feasibility (from the code, to be confirmed in step 1)

- The unit tests read JSON fixtures and use only the standard library.
- No lab gate uses `unshare`, `sudo`, `ip link` or `tc` (checked by
  grep). They use `127.0.0.1` ports and Unix sockets, which hosted
  `ubuntu-latest` runners allow.
- Only `chaos.py` needs namespaces. It stays local.
- The gates build `werewolfd` with `--locked --offline`. CI must fetch
  dependencies first (`cargo fetch --locked`).
- **Unknown:** wall-clock time and timing flakiness on shared runners.
  Step 1 measures this before any job is made required.

## 3. Proposed jobs (additions only; existing jobs unchanged)

1. **`python-unit`**: runs `python3 -B -m unittest discover -s
   tests/integration -p 'test_*.py'`. Blocking.
2. **`acceptance`**: runs `cargo fetch --locked`, then
   `python3 -B tests/integration/run.py`. Blocking after step 1 shows it
   is stable.
3. **`lab-gates`**: a matrix with one job per gate, each with its own
   timeout. Non-blocking at first (`continue-on-error`). Each gate is
   promoted to blocking only after N consecutive green runs (D4).
4. **`audit`**: `cargo audit` with an explicit ignore list that matches
   the documented acceptances (today: RUSTSEC-2026-0386, tokio 1.52.3,
   Stage33). A new advisory fails the job. Runs on PRs, plus a weekly
   schedule (D5).

Each job uploads its stdout JSON result line as an artifact. No job
uploads private identity keys: `--keep-temp` is never used in CI.

## 4. Invariants

- No production source, harness rule, test assertion or bound changes.
  If a gate needs a change to run in CI, that is a separate,
  owner-reviewed PR.
- Pin actions by version as the workflow does today (`actions/checkout@v4`
  and so on).
- `permissions: contents: read` stays. No secrets, no network beyond
  crates.io and the GitHub action sources.
- No live wolves, no `scripts/` gates, no `tests/stage22_private`.
- The audit ignore list lives in one file. Each entry cites the record
  that accepted it.

## 5. Steps

1. **Measure (no merge):** a throwaway branch runs the new jobs 5 times
   on hosted runners. It records duration and failures per job.
2. **Implement:** one PR adds the jobs in the §3 states, with a short
   record of the step 1 numbers.
3. **Promote:** after D4 is met, a one-line PR per gate removes
   `continue-on-error`.

## 6. Owner decisions

| # | Decision | Options | Recommended |
|---|---|---|---|
| S40-D0 | Approve Stage40 as scoped | approve, amend, reject | approve |
| S40-D1 | Python unit tests blocking from the start | yes, no | yes |
| S40-D2 | Acceptance (`run.py`) in CI | blocking after step 1; non-blocking; not at all | blocking after step 1 |
| S40-D3 | Lab gates in CI | all ten as a matrix; a subset; none | all ten, non-blocking first |
| S40-D4 | Promotion rule for a gate | N consecutive green runs (N = 10); owner decides per gate | N = 10 |
| S40-D5 | `cargo audit` | on PRs + weekly; weekly only; not at all | PRs + weekly |
| S40-D6 | Step 1 throwaway branch pushes to GitHub (runs on hosted runners) | yes; no (measure locally only) | yes |

**Not authorised by this spec:** any workflow change, any test or harness
change, tokio promotion, rc.3, release certification.
