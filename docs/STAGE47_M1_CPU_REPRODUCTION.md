# Stage47 M1 — CPU under descriptor exhaustion, reproduced

**M1: P2 and P3 CONFIRMED, P3 narrowed to TCP.**

- **P2:** the Stage42 `cpu_flooded_pct` (a 5 s window after the flood)
  does not compare builds reliably; the new cumulative metric
  (`cpu_seconds_flood`, Stage47 T1) does: the earlier build's `fd:tcp` runs
  agree within 0.05 s.
- **P3:** an encrypted-TCP Fang out of descriptors costs **12% more CPU**
  per episode on `main` than before Stage46 M2: 1.82 s mean (1.68–1.88)
  against 1.63 s (1.60–1.65); the ranges do not overlap. A QUIC Fang shows
  **no difference**: 0.85 s against 0.84 s.

The QUIC result weakens the survey's leading guess (§4 of the survey): QUIC
dials go through the same race and task machinery and show no cost. On
TCP the extra 0.19 s over about 700 races (two per failed setup, the
Stage32 F1 retry) is about 0.27 ms per race, far more than spawning a
task. M2 tests O1 as approved, but it may not close the gap.

The only change is tooling (T1). No production source changed.

## 1. Baseline

- `main` `685f760` (PR #145 merged). Lab debug `werewolfd` `e294de2b…`,
  as in Stage46 M3.
- Before Stage46 M2: `e897b31`, built from a temporary worktree
  (`target/stage47-m1/wt`, kept for the M2 A/B) as `bdd0327a…`. A debug
  build's hash depends on its build path.
- Owner decisions S47-D0–D3 as recommended: R1 correct the record, T1 the
  cumulative metric, O1 the single-address fast path kept only if
  measured.

## 2. Change: T1

`tools/stage42/fang_flood.py`: `fd:*` results gain `cpu_seconds_flood`,
B's CPU time from the start of the flood to the end of the hold (before
the release); `quota:*` results gain it from the flood's start to the
release. `cpu_flooded_pct` is unchanged. The README and the module help
say which to compare. The same tool file was copied into the worktree so
both builds were measured identically.

## 3. Method

`fang_flood.py fd:tcp fd:quic --nofile 64`, five rounds, each round
`main` then the earlier build, 2026-11-11 14:36–14:51 UTC. Results in
`target/stage47-m1/` (`main-N.json`, `pre-N.json`, `meta.txt`).

## 4. Results

| Scenario | Build | `cpu_seconds_flood` | mean | `cpu_flooded_pct` (5 s) | B's log | Flood |
|---|---|---|---|---|---|---|
| `fd:tcp` | `main` | 1.84, 1.86, 1.88, 1.68, 1.86 | **1.82** | 2.0–2.4% | 872–1,828 bytes | 13.2–30.3 s |
| `fd:tcp` | before M2 | 1.60, 1.65, 1.63, 1.62, 1.64 | **1.63** | 1.6–2.0% | 935 bytes every run | 28.7–30.8 s |
| `fd:quic` | `main` | 0.84, 0.83, 0.85, 0.93, 0.80 | **0.85** | 1.4–2.0% | 295 bytes | 5.5 s |
| `fd:quic` | before M2 | 0.80, 0.96, 0.73, 0.92, 0.80 | **0.84** | 1.6–2.0% | 295 bytes | 5.5 s |

- Every run: the daemon stayed up, recovered, and served a legitimate
  echo after the release.
- `fd:tcp` on `main`, run 4: the flood ended early (13.2 s, 574
  connections; the client stops after ten timeouts) and used the least
  CPU, 1.68 s. Without it the `main` range is 1.84–1.88.
- B's log on `main` varies because two failure classes (`dial` and
  `handshake`) each get a line per 10 s (Stage46 M2 §3); before M2 one
  class did.
- This round's 5 s figures happen to sit close together (2.0–2.4% against
  1.6–2.0%); Stage46 M3 saw 0.8–8.8% from the same build. The cumulative
  figure is the one to compare.

## 5. Consequences for M2

- O1 (single-address fast path) as approved, measured with this A/B, and
  kept only if `main`'s `fd:tcp` mean comes within the earlier build's
  spread (1.60–1.65 s).
- If it does not, M2 records O0 (accept, about 12% more CPU per
  exhaustion episode on encrypted TCP, none on QUIC, none in steady
  state), drops O1, and applies R1 with those figures.
