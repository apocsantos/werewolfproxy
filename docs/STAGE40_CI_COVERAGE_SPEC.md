# Stage40 — local CI coverage (spec and record)

**Status: IMPLEMENTED in this PR, for owner review.** The first draft of
this spec targeted GitHub Actions. The owner then decided **not to use
GitHub for CI** (2026-11-09), and chose a local script plus the pre-push
hook. PRs on GitHub continue as before. The existing
`.github/workflows/rust.yml` is unchanged.

Source: Stage39 survey §11 (testing gaps) and §13.3 (recommendation 3).

## 1. Problem

These checks ran only at stage gates, by hand:

| Check | Size | Needs |
|---|---|---|
| Python unit tests (`tests/integration/test_*.py`) | 64 tests, ~2 s, stdlib only | Python 3.11+ |
| Acceptance (`tests/integration/run.py`) | 12 assertions | Rust build, loopback TCP/UDP, Unix sockets |
| Ten lab gates (`tests/integration/*_gate.py`) | activation, auto_sync, death, discovery, governance, provider, routed_fang, service, service_discovery, sync | as acceptance |
| Advisory audit (`cargo audit`) | — | `cargo-audit` |

A regression in the authorization matrices, transport policy or the lab
gates was therefore caught only at the next stage gate. The Stage32 chaos
campaigns (`chaos.py`, namespaces, hours) stay out of scope.

## 2. Design (owner decisions, 2026-11-09)

- **`scripts/ci-local.sh`** runs the checks in sequence. The gates
  allocate loopback ports, so they must not overlap.
  - `--fast`: the Python unit tests and `cargo audit` (~5 s).
  - Default (full): `--fast` plus acceptance and the ten lab gates
    (~6 min with builds cached).
  - Each run writes `meta.txt` (commit, dirty files, tool versions),
    `summary.tsv` and one log per check to `target/ci-local/<UTC>/`.
    The script exits nonzero if any check fails.
- **The pre-push hook** (`scripts/install-git-hooks.sh`) adds
  `ci-local.sh --fast` after its existing steps. The full set runs on
  demand, before merges, and at stage gates.
- **`.cargo/audit.toml`** is the single ignore list. Each entry cites the
  record that accepted it (today only RUSTSEC-2026-0386, tokio 1.52.3,
  Stage33). The audit runs with `--deny warnings`, so a new unsound,
  unmaintained or yanked advisory also fails it.

The first draft's decisions S40-D0–D6 assumed GitHub Actions. D2–D6 (job
promotion, hosted runners) no longer apply. D1 carries over: the unit
tests block, now through the hook.

## 3. Invariants

- No production source, harness rule, test assertion or bound changes.
- No live wolves, no `scripts/*-gate.sh`, no `tests/stage22_private`.
  The gates kill only their own process groups.
- `--keep-temp` is never used, so no private test identity keys are kept.

## 4. Evidence

### 4.1 Timing (step 1, before the script)

Five sequential repetitions of all 13 checks on DarkSide (`main`
`e86a5db`, rustc 1.96.0, Python 3.13.5). Results:
`target/stage40-measure/20261109T100716Z/`.

| Check | Runs | Seconds (min–max) | Notes |
|---|---|---|---|
| python-unit | PASS 5/5 | 2.0–2.2 | |
| audit | PASS 5/5 | 1.1–2.3 | |
| acceptance | PASS 5/5 | 28–130 | first run includes its own build |
| gate-routed_fang | PASS 5/5 | 196–199 | the longest check; runtime, not build |
| gate-auto_sync | PASS 5/5 | 48–151 | first run includes the `lab-faults` build |
| gate-sync | PASS 5/5 | 23.8–23.9 | |
| gate-death | PASS 5/5 | 19.7–20.0 | |
| gate-provider | PASS 5/5 | 14.7–14.8 | |
| gate-service | PASS 5/5 | 10.9–11.0 | |
| gate-governance | PASS 5/5 | 7.1 | |
| gate-discovery | PASS 5/5 | 6.4–6.6 | |
| gate-activation | PASS 5/5 | 5.6–5.7 | |
| gate-service_discovery | PASS 5/5 | 5.1–5.2 | |

A warm repetition took ~370 s, the first 577 s; a cold workspace build
took 112 s. No check failed and the gates varied by under 3 s.

### 4.2 Script checks

- `--fast`: PASS, rc 0.
- Negative audit check: with RUSTSEC-2026-0386 removed from the ignore
  list, the audit FAILS and the script returns rc 1. The list was
  restored.
- An unknown argument returns rc 2 with a usage line.
- Full run: see §4.3.

### 4.3 Full run

- **First run** (`target/ci-local/20261109T135742Z`): 12 of 13 PASS.
  Acceptance FAILED its source-preservation check because this spec was
  edited during the run. That is the check working as intended: the full
  set must run on a tree nobody is editing.
- **Rerun** (`target/ci-local/20261109T140401Z`, tree untouched): **13 of
  13 PASS**, rc 0, 368 s.

## 5. Not authorised

GitHub Actions changes, tokio promotion, rc.3, release certification.
