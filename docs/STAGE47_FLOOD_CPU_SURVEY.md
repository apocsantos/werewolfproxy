# Stage47 — CPU under descriptor exhaustion (survey)

**Status: survey, for owner review.** Docs only. No production source,
harness, dependency or verdict changes. The probe runs below used a
scratch script and a temporary worktree, both removed; their results are
in `target/stage47-survey/`.

Stage46 M3 (§5) recorded that while an encrypted-TCP Fang's daemon is out
of file descriptors (`tools/stage42/fang_flood.py fd:tcp --nofile 64`) it
uses 0.8–8.8% of a core after Stage46 against 1.2–1.8% before, and
`KNOWN_LIMITATIONS.md` states it as uninvestigated. This survey measures
it properly first. **The finding is much smaller than recorded:** the
daemon spends about 11% more CPU over the whole episode (1.83–1.86 s
against 1.65–1.66 s), and nothing in the steady state. The 0.8–8.8% came
from where a 5 s sample falls relative to the end of the work.

## 1. Baseline

- `main` `bd0bcf1` (PR #144 merged, Stage46 CLOSED).
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18, rustix 1.1.6.
- Tag `v1.0.0-rc.2` frozen. S3-P2 and S2-P4 stay EXCLUDED.

## 2. What the Stage42 tool measures (CONFIRMED from code)

`scenario_fd` floods the Fang with up to 600 connections, which it keeps
open, waits 3 s, then takes `cpu_flooded_pct` as the daemon's CPU over the
next **5 s** (`cpu_percent`, utime + stime from `/proc/PID/stat`). So the
figure is not the flood's cost: it is the share of a 5 s window that
happens to catch the tail of the work. The flood itself lasts 28–38 s, and
how much work is left when it stops depends on timing. A build that ends
the flood with more queued work scores higher, whatever its total.

## 3. Probe (2026-11-11)

The same lab and flood (`fd:tcp`, `nofile` 64, B runs the Fang), with
cumulative CPU recorded from the start of the flood, the 5 s window as the
tool takes it, a 20 s window after that, and the setup failures counted
from B's kept log (each line plus its "N more"). Four runs per build:
`e294de2b…` (`main`, Stage46) and `5bdf5519…` (`e897b31`, just before
Stage46 M2, built in a temporary worktree; a debug build's hash depends
on its build path, so it differs from the Stage46 A/B's `d511e5de…` of the
same commit).

| Build | CPU s, flood | CPU s, flood to end of hold | 5 s window | 20 s window | Failed setups | Flood |
|---|---|---|---|---|---|---|
| before Stage46 | 1.24–1.52 | **1.65–1.66** | 1.4–2.0% | 0.1–0.3% | 338–442 | 30.3–30.8 s |
| Stage46 | 1.40–1.65 | **1.83–1.86** | 2.2–6.8% | 0.1% | 341–389 | 27.8–30.8 s |

(Run 1 of each has no cumulative figure; the table's ranges for those
columns are from runs 2–4.)

- **No steady-state cost.** With the 540 connections still held and the
  daemon at its descriptor limit, both builds use 0.1–0.3% over 20 s.
- **About 11% more CPU per episode**, for a similar number of failed
  setups: roughly 5.2 ms per failed setup against 4.9 ms.
- **The 5 s figure varies with timing:** 2.2–6.8% here on the Stage46
  build (0.8–8.8% in Stage46 M3), while the totals agree within 2%.
- `strace -c` during the 20 s hold recorded nothing useful: the daemon is
  idle then. `perf` is not installed.

## 4. Where the extra 11% may come from (code reading, not measured)

Per failed setup, Stage46 added:

- `transport/dial.rs` `race`: a `JoinSet` and a spawned task per attempt,
  even for a single literal address, and a `tokio::select!` loop;
- `connect_tcp`: a parse of the address text (cheap);
- under exhaustion, the failures now fall into two classes (`dial` and
  `handshake`), so two limiters and two lines per 10 s instead of one
  (bounded; a few lines).

Nothing in the session itself changed. The Stage32 F1 retry still runs the
whole attempt twice, as before.

## 5. Problems

- **P1, the record overstates it.** `KNOWN_LIMITATIONS.md` and the
  Stage46 closure state 0.8–8.8% against 1.2–1.8%; the measured cost is
  about 11% of total CPU per episode, with no steady-state cost.
- **P2, the Stage42 metric is timing-sensitive.** `cpu_flooded_pct` is a
  5 s window after the flood, so it compares when work ends, not how much
  there was. Every later flood comparison (Stage43–46) used it.
- **P3, a small per-dial overhead** of unknown cause, probably the race's
  task machinery, paid by every dial, including the single literal address
  every certified path uses.

## 6. Options

### Record (P1)

- **R1:** replace the Stage46 limitation with the measured figures (about
  11% more CPU per exhaustion episode, none in steady state), citing this
  survey.
- **R0:** leave it.

### Metric (P2)

- **T1:** add `cpu_seconds_flood` (cumulative, from the flood's start to
  the end of the hold) to `fang_flood.py`'s `fd:*` and `quota:*` results.
  Keep `cpu_flooded_pct` unchanged, so earlier results stay comparable.
- **T0:** leave the tool.

### Overhead (P3)

- **O1, single-address fast path:** `race` with one address calls the
  attempt directly, with no `JoinSet`, task or `select!`. Behaviour is the
  same; a literal address is still one attempt with the same error. Kept
  only if an A/B with T1 shows it closes the gap.
- **O2, profile first:** install `perf` (owner action) and find the cause
  before changing code.
- **O0, accept:** 11% more CPU per exhaustion episode, recorded.

## 7. Recommendation

**R1 + T1 + O1.**

- R1 corrects a record that is wrong by a factor of 3–5.
- T1 makes the Stage42 floods measure what they claim; it is additive.
- O1 is a small, behaviour-preserving change aimed at the likeliest
  cause and at the case every certified path uses. It is accepted only on
  a measured improvement; otherwise O0 is recorded and O1 dropped.

## 8. Proposed Stage47 contract (for owner review)

- **M1, reproduction:** `fd:tcp` and `fd:quic` at `nofile` 64, 5 runs each
  on `main` and on `e897b31`, with T1's cumulative metric (T1 lands in M1
  as tooling). Confirms P2/P3, and says whether `fd:quic` (whose dial
  creates no socket per session) shows any difference.
- **M2, fix:** O1 with a unit test that a single address is one attempt
  with no delay and the same error; R1 in `KNOWN_LIMITATIONS.md`. The same
  A/B; O1 kept only if the Stage46 build's cumulative CPU comes within the
  run-to-run spread of the pre-Stage46 build.
- **M3, regression:** the Stage46 matrix ×1, Stage42 floods ×3 with T1,
  Stage32 S2, S3, S4 P0 (every literal dial goes through the fast path),
  `ci-local` full.
- **M4:** `KNOWN_LIMITATIONS` and closure.

No wire, trust, Target Policy, receiver or dependency change.

## 9. Owner decisions

| ID | Decision | Options | Recommended |
|---|---|---|---|
| S47-D0 | Approve Stage47 as scoped (§8) | approve, amend, reject | approve |
| S47-D1 | Record | R1 correct to the measured figures; R0 leave | R1 |
| S47-D2 | Flood CPU metric | T1 add a cumulative metric, keep the old; T0 leave | T1 |
| S47-D3 | Per-dial overhead | O1 single-address fast path, kept only if measured; O2 profile first (needs `perf`); O0 accept | O1 |
