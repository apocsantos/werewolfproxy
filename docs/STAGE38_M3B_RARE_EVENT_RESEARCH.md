# Stage38 M3b — rare-event inference research

**Classification: M3B-B, a valid method at an impractical sample size.**

- **No new method improves on C6.** Of the new families:
  - R2 (block bootstrap-t) and R3 (studentised randomisation) fail
    false-acceptance control in every generator family.
  - R1 (betting confidence bounds) and its empirical-Bernstein variant
    are valid but need *more* data than C6. The Bernstein variant never
    rejects at all, even at 64 quartets.
- **C6 is confirmed** across the three generator families (logit-normal,
  mixtures with very bad blocks, shared or abruptly changing host
  conditions), under the corrected coverage rule. It still needs about
  48 quartets (61–67 h) at δ = 7.5 pp for a baseline of 4% or less, and
  more at 6%.
- **This is a mathematical limit, not a tuning problem.** Without an
  assumption limiting how failures cluster within a block, no valid test
  can declare non-inferiority at 7.5 pp with fewer than 39 candidate
  blocks (20 quartets), even if no failure is observed (section 5).
- **`NO_PRACTICAL_VALIDATED_METHOD`.** Comparative certification stays
  suspended (the owner's option a). Nothing here approves D4 or D5, and
  nothing is authorised.

> **Closure (2026-11-03):** the final architectural disposition, and
> the scientific qualifications that govern this report's use of
> "valid", are recorded in [STAGE38_M3B_CLOSURE.md](STAGE38_M3B_CLOSURE.md).
> The results below are unchanged.

These are simulation findings, not observations of Werewolf. The M3
report and its results are unchanged.

## 1. Baseline and provenance

- **Baseline:** `main` `1ea0d0c55bb1b02c9c342f1115c9dabf5b93fbab` (#106
  merged). tokio is 1.52.3; S3-P2 is EXCLUDED; the tag `v1.0.0-rc.2`
  is unchanged (object `ec10180`, commit `b192158`).
- **Tooling:** the M3 environment (Python 3.13.5, numpy 2.3.4, scipy
  1.16.3, statsmodels 0.14.5 for tests). New code: `s38/m3b.py`,
  `s38/m3b_scenarios.py`, `s38/m3b_runner.py`, `s38/m3b_analysis.py`,
  `s38/m3b_summarise.py`, `tests/test_m3b.py`. The M3 modules are
  unchanged except for being imported.
- **Frozen plan:** `tools/stage38_statistics/M3B_VALIDATION_PLAN.md`,
  committed (`920716d`) before any M3b estimator existed.
- **Runs** (4 workers; independent root seeds):

| Stage | Config | Seed | Cells × replicates | Methods | Wall time |
|---|---|---|---|---|---|
| Screening (Phase B) | `m3b_screening.json` | 38101 | 848 × 2,000 | C0, C2, C6, R1, R1EB, R2, R3 | 7,011 s |
| Size sweep | `m3b_power.json` | 38104 | 120 × 5,000 | C6, R1, R1EB | 1,347 s |
| Confirmation (Phase C) | `m3b_confirmation.json` | 38102 | 864 × 20,000 | C6 | 3,425 s |
| R1 null confirmation | `m3b_r1_null.json` | 38107 | 149 × 20,000 | R1 | 38,740 s |
| E1/E2 hierarchy | `m3b_hierarchy.json` | 38105 | 62 × 20,000 | C6, R1 | 3,006 s |
| Replacement policies | `m3b_replacement.json` | 38106 | 240 × 10,000 | C6 | 470 s |

- **Deviation from the frozen plan:** the plan named only seeds 38101–38103.
  The extra stages use new, independent seeds 38104–38107, and seed
  38103 (the 80,000-replicate rerun) was never needed, because no
  coverage cell was inconclusive in confirmation.
- **R1 confirmation subset:** this cost about 11 h even restricted to
  null cells at 48 quartets. The subset was fixed for cost before any
  confirmation result (see `r1_null` in `m3b_scenarios.py`).
- **Results:** per-cell CSVs stay in `target/stage38-m3b/`; their SHA-256
  values are in `results/m3b_SHA256SUMS`. The small summaries are in
  `results/m3b_*.csv`.

## 2. Validation-contract corrections (frozen before testing)

1. **False acceptance:** in every null cell (RD = δ, δ + 2.5 pp,
   δ + 7.5 pp), the one-sided 95% Monte Carlo **upper** bound of P(U < δ)
   must be at most 0.05.
2. **Coverage is now three-way:** a cell is *adequate* if the Monte Carlo
   **lower** bound is at least 0.95, *under* if the upper bound is below
   0.95, and *inconclusive* otherwise. A method qualifies only if every
   cell is adequate.

   M3's rule (Monte Carlo upper bound at least 0.95) shows only that
   coverage is not demonstrably below 95%; it is replaced. Re-reading
   M3 under the new rule: C6's minimum coverage of 0.988 at 20,000
   replicates has a lower bound of about 0.987, so it is adequate, and
   M3's conclusions about C6 stand. The methods M3 rejected fail both
   rules.
3. **Degenerate cases:** the M3 rule applies to the new methods too.
4. **Power:** at least 0.80 at RD = 0 for baselines of 2–6% and G1 up
   to σ = 1.5.
5. **Practicality (pre-registered and provisional):** at most 16
   quartets.
6. **E2:** fixed-sequence gatekeeping after E1.

## 3. Research hypotheses

- **R1:** a finite-sample bounded-variable bound on block proportions
  stays valid under any within-block dependence. The question is whether
  its adaptivity to small observed variance beats C6.
- **R2:** studentising with a bootstrap corrects the skewness that broke
  C2 and C3 in M3.
- **R3:** the balanced ABBA/BAAB assignment supports useful
  randomisation inference.

## 4. Generator families (`s38/m3b.py`)

The E1 model is logit P = μ[arm, seed] + τ·g(t) + B_b + w[b, r]. E2 is
modelled conditionally on not-E1, as logit P = ν[arm, seed] + λ₂·B_b
(λ₂ ∈ {0, 1}: E2 independent of, or sharing, the block state). μ and ν
are calibrated so that the target E1 and E2 rates hold **marginally**.

| Family | Block latent B_b | Represents |
|---|---|---|
| G1 | u ~ N(0, σ_arm²) (M3) | persistent sandbox state |
| G2 | u + 3·J, J ~ Bernoulli(π), π ∈ {0.05, 0.15} | occasional very bad blocks |
| G3ar | u + e_t, e a shared AR(1) host environment over consecutive blocks, ρ ∈ {0.5, 0.9}, SD 1 | slowly varying host load or persistent kernel state |
| G3step | u + 1·[t ≥ T], T ~ U(0, 1) per experiment | abrupt environmental deterioration |

- **Both arms** share the G3 environment, and treatment definitions are
  identical across families.
- **Calibration:** quadrature over every normal component of B, w and
  the campaign positions; verified to 1e-8, and by Monte Carlo
  (section 7).

## 5. Mathematical analysis (Phase A)

### 5.1 A floor for every method that does not restrict within-block dependence

**Setting.** Blocks are independent across the experiment, and within an
arm their proportions are i.i.d. on [0, 1], with any within-block
dependence. This is the setting a "distribution-free block-level"
method assumes.

**Claim.** Fix δ ∈ (0, 1) and n_B candidate blocks. If a test is level α
for H0: RD ≥ δ over this whole class, and it declares non-inferiority
when every block of both arms has no E1 event, then (1 − δ)^n_B ≤ α.

**Proof.** Take the null law in which every control block has
proportion 0, and every candidate block has proportion 1 with
probability δ and 0 otherwise (all-or-nothing legs). Its RD is δ, so it
lies in H0. All-zero data has probability (1 − δ)^n_B under it, and the
test rejects on that data, so its size is at least (1 − δ)^n_B. ∎

**Consequence** (`m3b.floor_blocks`, tested):

| δ | Candidate blocks needed | Quartets |
|---|---|---|
| 5 pp | 59 | 30 |
| 7.5 pp | 39 | 20 |
| 10 pp | 29 | 15 |

These are the *best case*: no event observed at all. Real data at a 2–6%
baseline needs more.

**What follows.** Any method practical at 16 quartets or fewer must make
an assumption that excludes such laws, for example a bound on block
dependence. The data cannot verify such an assumption with a few rare
events, and the A/A pilot could not either (spec section 7). That is
why "practical" and "valid without unverifiable assumptions" cannot both
hold here.

### 5.2 The candidate families

| | R1 betting (Waudby-Smith & Ramdas 2023) | R1EB empirical Bernstein (Maurer & Pontil 2009) | R2 block bootstrap-t | R3 studentised randomisation |
|---|---|---|---|---|
| Estimand | RD on block proportions | as R1 | RD | average effect over quartets |
| Bound | U = U_B − L_A, L = L_B − U_A, each arm at α/2 (Bonferroni); a predictable plug-in bet over blocks in campaign order; grid step 0.0025, rounded outward | each arm: mean ± √(2V ln(2/α′)/n) + 7 ln(2/α′)/(3(n−1)) | Hall's bootstrap-t: blocks resampled within arm (999); U = d − t*₍α₎·se | balanced orientation patterns, sign flip of the quartet differences minus δ, studentised |
| Valid when | blocks independent, and within an arm every block's conditional mean equals the arm mean (i.i.d. suffices). Finite-sample. | blocks i.i.d. Finite-sample. | asymptotic (second-order) only | sharp null only; the weak null only asymptotically (Wu & Ding 2021) |
| Under G3 (dependent or nonstationary blocks) | **not guaranteed**: the supermartingale argument needs each block's conditional mean ≥ m, which drift breaks | not guaranteed | not guaranteed | not guaranteed |
| Zero events | the bound is defined (all-zero gives U ≥ 1 − α′^(1/n), tested); the common rule makes it DEGENERATE | defined | se* = 0 resamples get t* = ±∞ or 0 by sign (so U = +∞ when the lower tail is infinite); se = 0 → DEGENERATE | a zero-spread statistic is ±∞ by the sign of the mean, 0 when the mean is 0 |
| Floor (5.1) | applies | applies, and the additive 7 ln(2/α′)/(3(n−1)) term alone is 0.26 at n = 48 | does not apply (not distribution-free), so it must fail somewhere in the class | as R2; and attainable p is 1/C(Q, Q/2): no rejection at Q = 4; 1/20 at Q = 6 |
| Phase A verdict | kept: valid, likely low power | kept as a variant; expected useless | kept: no finite-sample guarantee, test empirically | kept with a limit: the constant-shift sharp null is incoherent for proportions in [0, 1] at a margin with many zero blocks (it implies negative control proportions); only the studentised weak-null version is tested, Q ≥ 6 (Q = 24 is beyond the 13,000-pattern limit: NA) |

## 6. New estimator implementations

The estimators are in `s38/m3b.py` (`r1_betting`, `r1_bernstein`,
`r2_bootstrap_t`, `r3_randomisation`). They are called through
`m3b.call`, which applies the M3 common degenerate rule. Each method
has its own random stream (R2's bootstrap): `SeedSequence(root,
spawn_key=(cell, chunk, 1000 + method index))`.

Cost per 1,000 replicates, both endpoints, at 8 quartets: R1 about
1.2 s (it grows with blocks: about 9 s at 48 quartets), R1EB under
0.01 s, R2 about 4 s, R3 about 8 s.

## 7. Unit and reference tests

`python -B -m unittest discover -s tests` gives **41 tests, all
passing**: the 23 M3 tests, unchanged, and 18 new.

- **Generators:**
  - quadrature calibration of E1 and E2 to 1e-8 in every family, with
    carry-over, asymmetry and drift;
  - Monte Carlo marginals (20,000 experiments) in every family;
  - E1/E2 exclusivity, and zero-rate E2;
  - infeasible cells refused.
- **Schedules:** identity without invalidation; completion
  probabilities for cap 2, cap 6, immediate and restart against exact
  formulas; positions distinct and ordered.
- **R1:**
  - the vectorised bounds against an independent scalar loop
    implementation;
  - coverage on i.i.d. Bernoulli and Beta data, never demonstrably
    below 95%;
  - the all-zero floor;
  - the empirical Bernstein formula.
- **R2 and R3:**
  - the bootstrap-t against a loop reimplementation using the same
    stream;
  - R3 against direct enumeration of the patterns;
  - R3 cannot reject at Q = 4.
- **Common:** the degenerate rule for the new methods; the floor; the
  hierarchy.

**The tests found one real defect in R3 before any run.** When every
quartet difference equalled the hypothesised value, the studentised
statistic was sign(0)·∞ = NaN. NaN compares false, so the p-value
became 0 and the test rejected. Fixed: a zero-spread statistic is 0
when its mean is 0. A test for it was added.

## 8. Screening results (Phase B, 2,000 replicates)

Maximum E1 false acceptance at δ = 7.5 pp, and the number of null cells
whose Monte Carlo upper bound exceeds 0.05
(`results/m3b_screening_error_control.csv`):

| Family | C0 | C2 | C6 | R1 | R1EB | R2 | R3 |
|---|---|---|---|---|---|---|---|
| G1 | 0.279 (91) | 0.214 (72) | 0.0045 (0) | 0 (0) | 0 (0) | 0.154 (71) | 0.114 (30) |
| G1 nuisance | 0.249 (64) | 0.125 (64) | 0.0025 (0) | 0 (0) | 0 (0) | 0.112 (64) | 0.090 (32) |
| G2 | 0.258 (91) | 0.206 (60) | 0.001 (0) | 0 (0) | 0 (0) | 0.187 (64) | 0.124 (26) |
| G3ar | 0.302 (93) | 0.262 (82) | 0.001 (0) | 0 (0) | 0 (0) | 0.222 (73) | 0.245 (36) |
| G3step | 0.242 (36) | 0.223 (27) | 0 (0) | 0 (0) | 0 (0) | 0.196 (27) | 0.175 (12) |

E1 coverage of U over all 848 cells (adequate / inconclusive / under):

| C0 | C2 | C6 | R1 | R1EB | R2 | R3 |
|---|---|---|---|---|---|---|
| 5 / 62 / 781 | 100 / 163 / 585 | 848 / 0 / 0 | 848 / 0 / 0 | 848 / 0 / 0 | 12 / 170 / 666 | 556 / 48 / 244 |

- **Rejected in screening:** R2 and R3, in every family. Their
  bootstrap and randomisation corrections do not repair skewed,
  zero-inflated block proportions with few blocks; the dependent G3
  families are the worst. C0 and C2 reproduce M3's failures.
- **No power at practical sizes:** R1 and R1EB never reject at 4–24
  quartets (power 0), as the floor in 5.1 predicts.

## 9. Confirmation results (Phase C, 20,000 replicates)

**C6 over 864 cells:**
- G1 at σ 0–2.5; G1 with asymmetry, carry-over, seed heterogeneity and
  drift;
- G2 (π 0.05, 0.15); G3ar (ρ 0.5, 0.9); G3step;
- at 32 and 48 quartets.

| Margin | Null cells | Max false acceptance | Max Monte Carlo upper bound (per family) |
|---|---|---|---|
| 5 pp | 552 | 0.00045 | 0.0008 (G1), 0.0003 (G1n), 0.0004 (G2), 0.0002 (G3ar), 0.0001 (G3step) |
| 7.5 pp | 552 | 0.0037 | 0.0044, 0.0018, 0.0027, 0.0033, 0.0020 |
| 10 pp | 240 | 0.0056 | 0.0059, —, 0.0036, 0.0065, 0.0040 |

C6's coverage is **adequate in all 864 cells**, for U and L, for E1 and
E2.

**R1, null cells at 48 quartets (149 cells):** the maximum false
acceptance is 0.00005 (upper bound 0.0002) at 7.5 pp, and 0.0015 (upper
bound 0.0020) at 10 pp. Coverage is adequate in all 149 cells.

Both methods meet the error-control and coverage criteria in every
family tested, **including G3, where neither has a guarantee**. That is
evidence about these models, not a proof for dependent blocks in
general.

## 10. Power and sample size against C6

E1 power at RD = 0 and δ = 7.5 pp, G1 (size sweep, 5,000 replicates; σ
0 / 1.5):

| p_A | Method | 16 Q | 24 Q | 32 Q | 48 Q | 64 Q |
|---|---|---|---|---|---|---|
| 2% | C6 | 0.03 / 0.09 | 0.68 / 0.65 | 0.99 / 0.94 | 1.00 / 1.00 | 1.00 / 1.00 |
| 2% | R1 | 0 / 0 | 0 / 0 | 0 / 0 | 0.91 / 0.78 | 1.00 / 1.00 |
| 4% | C6 | 0.01 / 0.08 | 0.26 / 0.36 | 0.73 / 0.65 | 1.00 / 0.95 | 1.00 / 0.99 |
| 4% | R1 | 0 / 0 | 0 / 0 | 0.00 / 0.01 | 0.72 / 0.41 | 1.00 / 0.89 |
| 6% | C6 | 0.00 / 0.06 | 0.11 / 0.23 | 0.39 / 0.45 | 0.92 / 0.79 | 1.00 / 0.94 |
| 6% | R1 | 0 / 0 | 0 / 0 | 0.01 / 0.01 | 0.62 / 0.28 | 0.99 / 0.68 |

R1EB has power 0 at every size up to 64 quartets. In screening, at 4–24
quartets R1 has power 0. C6 has at most 0.08 at 16 quartets, and
0.10–0.67 at 24.

C6's worst-cell power at 48 quartets (confirmation, minimum over each
family's cells, σ up to 2.5):

| p_A | G1 | G1 nuisance | G2 | G3ar | G3step |
|---|---|---|---|---|---|
| 2% | 0.990 | 0.996 | 0.995 | 0.996 | 0.998 |
| 4% | 0.884 | 0.917 | 0.901 | 0.930 | 0.930 |
| 6% | 0.738 | 0.766 | 0.750 | 0.769 | 0.790 |

**The sample-size picture is unchanged from M3.** At δ = 7.5 pp:

| p_A | C6 needs | Campaign |
|---|---|---|
| ≤ 2% | about 32 quartets | 41–45 h |
| ≤ 4% | about 48 quartets | 61–67 h |
| 6% | about 64 quartets | 81–90 h |

R1 needs more than C6 at every size. Nothing reaches 80% power at 16
quartets or fewer; the floor in 5.1 makes that impossible without an
unverifiable assumption.

## 11. E2 and the hierarchy

E2 is a clean early reset, excluding E1; its baseline is 12%, and the
provisional margin is δ₂ = 10 pp. Hierarchy cells are at 48 quartets,
with λ₂ ∈ {0, 1}, p_A ∈ {2%, 4%} and σ ∈ {0, 1.5}, 20,000 replicates
(`results/m3b_hierarchy.csv`).

| Scenario (E1 RD, E2 RD) | Expected | C6 E1 NI | C6 overall NI | R1 overall NI |
|---|---|---|---|---|
| (0, 0): neither worsens | NONINFERIOR | 0.99 | up to 0.947 | up to 0.898 |
| (0, +10): E1 fine, E2 at its margin | not NONINFERIOR | 0.99 | 0.000 | 0.000 |
| (0, +15): E2 worse | not NONINFERIOR | 0.99 | 0.000 (INFERIOR 0.014) | 0.000 |
| (+7.5, −5): E1 at margin, E2 improves | not NONINFERIOR | 0.000 | 0.000 | 0.000 |
| (+7.5, +10): both at margin | not NONINFERIOR | 0.000 | 0.000 | 0.000 |
| (+10, +15): both worse | not NONINFERIOR | 0.000 | 0.000 | 0.000 |

- **Family-wise error:** the largest Monte Carlo upper bound of a false
  overall NONINFERIOR, over every cell where either endpoint's null is
  true, is **0.0007** (C6) and 0.0002 (R1).
- **No compensation:** an improving E2 never rescues a marginal E1, by
  construction and in simulation.
- **E2 power:** across the confirmation cells (E2 RD = 0), C6's
  worst-cell E2 non-inferiority power at 48 quartets is 0.76–0.81 by
  family. So the overall
  hierarchy is below 80% in some cells even where E1 is.
- **Sparse cells:**
  - *E1 at 0.5%:* C6 declares E1 non-inferior in every experiment at
    RD = 0, and never at RD = δ.
  - *No E2 event at all:* E2 is DEGENERATE under the common rule, so
    the overall result can **never** be NONINFERIOR (0 of 20,000), even
    though E1 passes in 98%. This follows from the existing rules, but
    it is a policy question for the owner: an endpoint that never occurs
    in either arm blocks certification.

## 12. Invalidity and replacement (a design study; V5 unchanged)

Completion probability (the experiment ends valid), G1 σ = 1 (host
model: G3ar ρ = 0.9), C6, 10,000 replicates (`results/m3b_replacement.csv`):

| Invalidation | Q | cap 2 (V5) | cap 6 | immediate, cap 2 | restart, 3 attempts |
|---|---|---|---|---|---|
| independent 0.1% | 48 | 0.999 | 1.000 | 0.999 | 0.995 |
| independent 0.5% | 48 | 0.922 | 1.000 | 0.921 | 0.764 |
| independent 1% | 48 | 0.679 | 0.994 | 0.692 | 0.376 |
| independent 5% | 48 | 0.002 | 0.103 | 0.002 | 0.000 |
| whole-quartet 1% / 5% | 48 | 0.985 / 0.541 | 1.000 / 0.983 | 0.987 / 0.541 | 0.946 / 0.235 |
| late-rising 1% / 5% | 48 | 0.664 / 0.002 | 0.991 / 0.057 | 0.668 / 0.001 | 0.165 / 0.000 |
| host-driven 1% / 5% | 48 | 0.222 / 0.002 | 0.733 / 0.019 | 0.225 / 0.001 | 0.070 / 0.000 |
| independent 1% | 8 | 0.994 | 1.000 | 0.994 | 0.976 |

**Selection bias** in the RD estimate at the null (RD = 7.5 pp), 48
quartets:

| Invalidation | Bias | False acceptance |
|---|---|---|
| independent, whole-quartet, late-rising | within ±0.5 pp | normal |
| host-driven 1% | −0.9 to −1.8 pp | 0.2–0.5% (normally about 0.3%) |
| host-driven 5% | −2.6 to −4.9 pp | 2.3% (cap 6, 217 valid experiments, upper bound 4.8%); 9% (cap 2, 1 in 11 valid) |

- **Why host-driven invalidation is dangerous:** it discards the
  quartets run under bad host conditions. Those raise the candidate's
  higher rate more, in absolute terms, so discarding them shrinks the
  observed difference and favours the candidate. The bias is toward
  **false acceptance**.
- **Where the data is too thin:** cells with very few valid experiments
  (one restart cell had a single valid experiment, which accepted) carry
  no rate information. They are reported, not counted.
- **Elsewhere:** with at least 1,000 valid experiments, C6's false
  acceptance is at most 0.004 (upper bound 0.007), coverage is adequate
  in all 202 such cells, and power at 48 quartets is 0.98 regardless of
  policy.

**Comparison of the policies:**

| Policy | Finding |
|---|---|
| **cap 2 (V5)** | fragile beyond about 24 quartets unless block invalidation is 0.2% or less |
| **cap 6** | completes far more often (99% at 1% independent invalidation), with no loss of error control under independent invalidation |
| **immediate replacement** | no gain over cap 2 in completion. It runs fewer total blocks when it fails early, because it abandons sooner. It keeps replacements close in time to the quartet replaced, which reduces drift exposure. |
| **restart** | the worst completion rate, and the strongest host-selection bias |
| **all policies** | cannot remove the selection bias of host-driven invalidation |

Whether invalidation is independent of host conditions is the critical,
unverified assumption. Any V5 change needs a new owner decision.

## 13. Failure cases and unresolved assumptions

- **Failures retained:**
  - every R2 and R3 null cell over 5%;
  - every under-coverage cell;
  - the host-driven bias;
  - the cells without enough valid experiments.
- **Theory:** C6 and R1 have no guarantee under dependent or
  nonstationary blocks (G3). Their G3 results are evidence for these
  models only. Even for independent blocks, C6's conservativeness rests
  on a variance bound and a normal approximation, not a finite-sample
  proof. R1's validity is finite-sample, but needs equal conditional
  block means within an arm.
- **Not evaluated:**
  - R1 at 32 quartets (where it has no power) in confirmation;
  - C6 at 64 quartets under G2 and G3;
  - other model families (for example invalidation that correlates with
    the treatment, or long-memory environments).
- **E2:** the zero-event endpoint rule (section 11) is a policy gap.

## 14. Recommended next step

1. **Classification: M3B-B.** The only valid methods (C6, and R1, which
   is worse) need multi-day campaigns. No new method is better than C6.
   Record `NO_PRACTICAL_VALIDATED_METHOD`.
2. **Keep comparative certification suspended.** Do not run the M4 A/A
   pilot, which would not change this conclusion.
3. **Owner decisions needed** if comparative certification is ever to
   be revisited:
   - whether a 61–90 h C6 campaign is acceptable;
   - whether to change V5 (a larger cap: cap 6 removes most of the
     completion fragility);
   - how to establish that invalidation is independent of host
     conditions (the selection-bias risk);
   - how a zero-event E2 should be treated;
   - whether to accept a margin of 10 pp (C6 about 24–32 quartets).

   None of these is recommended here. Each trades certification rigour
   for convenience, and is the owner's decision.
4. **Otherwise:** dependency changes such as tokio 1.54.1 keep being
   judged under the existing absolute rules and the owner's
   architectural review. This research does not approve any method.

Not authorised by this report: D4 or D5 approval, the A/A pilot, a
candidate trial, harness or V5 changes, tokio promotion, and release
certification.
