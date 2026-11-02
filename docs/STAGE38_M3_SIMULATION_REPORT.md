# Stage38 M3 — statistical simulation report

**Result: `NO_VALIDATED_PRIMARY_METHOD` that is certification-ready.**

- **Error control:** of the eight candidates, only **C6** (block-level
  effective sample size) controls false acceptance and coverage across
  the whole required region.
- **Power:** C6 reaches 80% power at δ = 7.5 pp only at about **48
  quartets** (192 blocks, 2,880 S3-P2 runs, about 61–67 hours of
  campaign) for a baseline E1 rate of at most 4%, and about 64 quartets
  at 6%.
- **Validity:** at those sizes the V5 replacement cap (two quartets)
  leaves an experiment valid only if fewer than about 0.2% of blocks are
  invalidated.
- **Classification:** under owner decision V2, C6 is technically
  interesting but not certification-ready.
- **The other seven methods are rejected** for false acceptance and
  coverage failures under block dependence. Most of them already fail
  at σ = 0.

These are simulation findings, not observations of Werewolf. They do
not approve D4 or D5, and they authorise nothing.

## 1. Provenance

- **Baseline:** `main` `bb87c8cff7f3e305cfac5840e617646fdab6e48f`
  (#103, #104 and #105 merged). tokio is 1.52.3; S3-P2 is EXCLUDED
  (`KNOWN_LIMITATIONS.md`). The tag `v1.0.0-rc.2` is unchanged (tag
  object `ec10180`, commit `b192158`).
- **Tooling:** `tools/stage38_statistics/` (this PR).
  - Python 3.13.5; numpy 2.3.4, scipy 1.16.3, statsmodels 0.14.5
    (tests only), pandas 3.1.0 (`requirements.txt`).
  - The packages run in a virtual environment under
    `target/stage38-m3/venv`, outside the repository.
  - They are not in Werewolf's runtime or Rust workspace.
- **Host:** Linux 6.12.113, 4 CPUs, one worker per CPU,
  `OMP_NUM_THREADS=1`.
- **Runs:**

| Stage | Config | Root seed | Cells | Replicates per cell | Wall time |
|---|---|---|---|---|---|
| Screening | `config/screening.json` | 38001 | 5,072 (16 infeasible, recorded) | 2,000 | 1,433 s |
| Size sweep | `config/power.json` | 38003 | 384 | 20,000 | 667 s |
| Confirmation | `config/confirmation.json` | 38002 | 5,856 | 20,000 | 11,751 s |
| Interim | `config/interim.json` | 38004 | 72 | 20,000 | 209 s |

The four root seeds give independent streams; every (cell, chunk) has its
own `SeedSequence` child. Full per-cell CSVs stay in `target/stage38-m3/`;
their SHA-256 values are in `tools/stage38_statistics/results/SHA256SUMS`
(screening `1dc25026…`, power `0de88885…`, confirmation `238abb55…`). The
summaries in `results/` are derived from them by `s38.summarise`.

## 2. Implementation

- **Design** (`s38/design.py`):
  - 15-run blocks in the harness seed order 2010, 2020, 2030, repeated
    five times;
  - balanced ABBA/BAAB quartets, with exactly half ABBA (V3);
  - whole-quartet replacement under an objective,
    outcome-independent invalidation process, capped at two replacement
    quartets (V5). A replacement takes the orientation of the quartet
    it replaces, so the analysed quartets stay balanced. An experiment
    beyond the cap is INVALID and is never counted as a statistical
    result.
- **Generator** (`s38/generator.py`):
  - logit P(E1) = μ[arm, seed] + τ·g(t) + u_block + w_run;
  - **block effect:** u ~ N(0, σ_arm²);
  - **carry-over:** w is a stationary AR(1) over the 15 runs with
    persistence φ and SD 1 (a modelling choice);
  - **drift:** g is none, linear (t − ½) or a step at the campaign
    midpoint;
  - **calibration:** μ is solved by Gauss–Hermite quadrature (64 nodes)
    so the target seed rates hold **marginally**, integrated over u, w
    and the design's campaign positions;
  - **outcomes:** each leg is E1, E2 (probability 0.12 among non-E1
    legs) or completed, so E1 and E2 are exclusive.
- **Estimand** (spec section 4): RD = ⅓ Σ_s (p_B,s − p_A,s).
- **Estimators** (`s38/estimators.py`), each vectorised, each answering
  for every threshold d0: is U < d0 and is L > d0 (one-sided α = 0.05).

| ID | Construction | Seeds | Block dependence | Small-sample handling |
|---|---|---|---|---|
| C0 | stratified Miettinen–Nurminen score, legs independent | strata, equal weights | none (diagnostic) | MN n/(n−1) |
| C1 | C0 with per-arm design effect = observed block-proportion variance ÷ binomial variance, floored at 1 | strata | estimated inflation | none beyond MN |
| C2 | Welch t on block proportions | blocks are seed-balanced | blocks as units | Welch–Satterthwaite df |
| C3 | paired t on quartet differences (B blocks − A blocks) | as C2 | quartets as units | df = Q − 1 |
| C4 | linear probability model with arm and seed terms; CR2 cluster-robust variance (Bell–McCaffrey) | covariates | blocks as clusters | Bell–McCaffrey df |
| C5 | randomisation test over every balanced orientation pattern, inverted under a constant shift | as C3 | the assignment itself | exact over the pattern set |
| C6 | C0 with each arm's effective size = its number of blocks (design effect 15) | strata | full within-block correlation | MN n/(n−1) |
| C7 | beta-binomial profile likelihood: shared overdispersion ρ, block means μ_A and μ_B; signed-root LR on a grid (μ step 0.0025, 31 ρ values) | not modelled | overdispersion | none (asymptotic) |

**Common degenerate rule** (all methods, tested): an experiment with no
E1 event, or with every leg an E1 event, is DEGENERATE and rejects
nothing. A method whose variance is zero is DEGENERATE. A numerical
failure is FAILED (C7: the MLE at the grid edge). Neither ever counts as
a rejection, and no method substitutes another interval. The unit tests
found an earlier draft in which C0 marked a zero-event experiment
DEGENERATE but still rejected at δ; that was fixed before any run.

**Cost** per 20,000 replicates: C0, C1, C3, C5 and C6 under 0.2 s; C2
and C4 under 0.25 s at 8 quartets; C7 about 3.6 s. Simulation itself
takes 1–20 s depending on size and carry-over.

## 3. Reference and unit tests

`python -B -m unittest discover -s tests` gives **23 tests, all passing**:

- **Design:** balance and reproducibility; block, seed and leg counts;
  equal seed weights; replacement (distinct slots, balance kept, the
  cap enforced, no replacement at 0%); attainable C5 thresholds.
- **Generator:**
  - marginal calibration: quadrature to 1e-9, and Monte Carlo over
    20,000 experiments, under asymmetric dependence, carry-over, seed
    heterogeneity and both drift kinds;
  - E1/E2 exclusivity, and the E2 rate;
  - true RD, and infeasible cells refused;
  - reproducible and distinct random streams.
- **References:**
  - **Restricted MLE:** checked against a numerical optimiser of the
    constrained likelihood in 4 cases.
  - **MN score:** checked against statsmodels at Δ0 = 0. At Δ0 ≠ 0,
    statsmodels 0.14.5's restricted estimate had a *lower* constrained
    likelihood than the optimiser's in our reference case (−33.5816
    against −33.5809), so the optimiser is the reference there.
  - **Welch and paired t:** checked against SciPy.
  - **CR2:** reduces exactly to the Welch variance without seed terms,
    with Bell–McCaffrey df = 2nb − 2.
  - **C4:** its point estimate equals the mean difference.
  - **Beta-binomial table:** checked against `scipy.stats.betabinom`.
- **Behaviour:** zero events (every method DEGENERATE); one-sided zero;
  a single event; all events; identical arms give no finding; swapping
  arms reverses the bounds (U ↔ −L); bounds are monotone in d0; C5
  cannot reach 5% with 4 quartets.
- **Findings:** non-inferiority, inferiority (L > δ), worsening (L > 0)
  reported together; any Layer A state other than `SATISFIED` blocks.

## 4. Grid and assumptions

**Supported by Stage34–37 evidence:**
- baseline E1 rate 2–6%;
- 15-run blocks in the fixed seed order;
- E2 about 12%;
- possible seed heterogeneity (seed 2030 had 3 of 5 baseline E1 events);
- campaign-level drift (Stage36).

**Modelling choices:**
- the logit-normal block effect and its σ grid;
- AR(1) carry-over with SD 1;
- the drift magnitudes (τ = ±0.5 linear, +0.5 step);
- a constant additive candidate effect across seeds;
- independent, outcome-independent invalidation.

| Dimension | Values |
|---|---|
| Baseline E1 rate p_A | 2%, 4%, 6% (sparse sensitivity: 0.5%, 1%) |
| True RD | −2, −1, 0, +2.5, +5, +7.5, +10, +15 pp (infeasible cells recorded and skipped) |
| Block σ | 0, 0.5, 1.0, 1.5 (mandatory, V1); 2.0, 2.5 (sensitivity) |
| Asymmetry | σ_B = σ_A, and σ_B = σ_A + 0.5 |
| Carry-over φ | 0, 0.5 |
| Seeds | equal; seed 2030 at 3× the others |
| Drift | none; linear +; linear −; midpoint step |
| Size | 2, 4, 6, 8 quartets (60–240 legs per arm); size sweep 4–64; confirmation 8 and 48 |
| Margins | 5, 7.5, 10 pp (every margin is evaluated on the same data) |
| Invalidation per block | 0, 5%, 10% |

- **Screening** (2,000 replicates):
  - **A:** a full factorial of rate × RD × σ × size;
  - **B:** a full factorial of rate × RD ∈ {0, 5, 7.5, 10 pp} × σ (all
    six) × {4, 8 quartets} × asymmetry × carry-over × seeds × drift;
  - **C:** invalidation;
  - **D:** sparse rates.
- **Confirmation** (20,000, an independent stream):
  - **F:** a full factorial of rate × RD ∈ {0, 5, 7.5, 10, 15 pp} × σ
    (all six) × {8, 48 quartets} × asymmetry × carry-over × seeds ×
    drift;
  - **G:** invalidation with linear drift.

No scenario was removed after a result was seen.

## 5. Criteria (owner decision V2), as applied

A method qualifies only if, in every required null cell (RD ≥ δ), the
one-sided 95% Clopper–Pearson Monte Carlo upper bound on false acceptance
P(U < δ) is ≤ 0.05, **and** the one-sided bounds' coverage is not
significantly below 95%: its Clopper–Pearson upper bound must be at
least 0.95.

A consequence: a test of exactly nominal size would fail the first
criterion at any number of replicates. Only a test whose true size is
below 5% by more than the Monte Carlo uncertainty can qualify. This is
how the criterion was set; it was not relaxed.

## 6. Screening results (2,000 replicates, root 38001)

Maximum false acceptance over the null cells and coverage, all groups
(`results/screening_error_control.csv`):

| | C0 | C1 | C2 | C3 | C4 | C5 | C6 | C7 |
|---|---|---|---|---|---|---|---|---|
| δ = 7.5: null cells | 2,488 | 2,488 | 2,488 | 2,488 | 2,488 | 2,488 | 2,488 | 2,488 |
| max false acceptance | 0.329 | 0.231 | 0.254 | 0.211 | 0.259 | 0.145 | **0.003** | 0.206 |
| cells clearly above 5% (CP lower > 0.05) | 1,850 | 1,224 | 1,508 | 1,445 | 1,546 | 425 | **0** | 1,626 |
| minimum coverage of U | 0.671 | 0.764 | 0.746 | 0.789 | 0.742 | 0.851 | **0.997** | 0.794 |
| minimum coverage of L | 0.715 | 0.834 | 0.900 | 0.906 | 0.890 | 0.850 | **0.994** | 0.887 |

At δ = 5 and 10 pp the picture is the same: the maximum false acceptance
is 0.11–0.32 for every method but C6, whose maximum is 0.002–0.003.

Maximum false acceptance at δ = 7.5 by block σ
(`results/screening_max_fa75_by_sigma.csv`):

| σ | C0 | C1 | C2 | C3 | C4 | C5 | C6 | C7 |
|---|---|---|---|---|---|---|---|---|
| 0 | 0.092 | 0.067 | 0.093 | 0.089 | 0.098 | 0.067 | 0.000 | 0.089 |
| 0.5 | 0.145 | 0.090 | 0.126 | 0.107 | 0.135 | 0.085 | 0.000 | 0.136 |
| 1.0 | 0.213 | 0.136 | 0.175 | 0.146 | 0.186 | 0.103 | 0.000 | 0.180 |
| 1.5 | 0.260 | 0.172 | 0.213 | 0.171 | 0.222 | 0.128 | 0.001 | 0.200 |
| 2.5 | 0.329 | 0.231 | 0.254 | 0.211 | 0.259 | 0.145 | 0.003 | 0.206 |

**Why the block-level methods fail too.** With rare events and block
effects, block proportions are strongly right-skewed. At p_B = 11.5% and
σ = 1.5, 41% of candidate blocks have no event and the skewness is about
2.0. A t-type upper bound on a right-skewed mean under-covers. An
independent loop-based reimplementation of C2 in one cell (4%, σ = 1.5,
4 quartets, RD = δ) gave 0.110 against the vectorised 0.120, within
Monte Carlo error. The failure is a property of the method, not the
code.

- **Even at σ = 0:** carry-over or asymmetric dependence alone pushes
  every method but C6 above 5% (the σ = 0 row includes φ = 0.5 and
  asymmetric cells).
- **Not much better with size:** the size sweep (20,000 replicates) shows
  C2 and C4 still above 5% at 64 quartets under σ = 1.5 (0.069).
- **C5:** attainable one-sided p-values over the balanced pattern set are
  1/2, 1/6, 1/20 and 1/70 for 2, 4, 6 and 8 quartets. It cannot reject
  at all with 2 or 4 quartets. At 6 and 8 quartets it accepts falsely up
  to 0.145, because its constant-shift working model does not hold for
  binary block proportions.
- **C7:** misspecified (logit-normal blocks, seeds not modelled); 16
  grid-edge failures (FAILED).

**Rejected after screening:** C0, C1, C2, C3, C4, C5 and C7. Each fails
false acceptance in hundreds of null cells, with Clopper–Pearson lower
bounds above 0.05, which is a clear failure and not Monte Carlo noise;
each also fails coverage. C6 was carried forward. C2 was also carried
forward, as a reference.

## 7. High-precision confirmation (20,000 replicates, root 38002)

C6 over group F (5,760 cells, 8 and 48 quartets, every σ, asymmetry,
carry-over, seed pattern and drift):

| Margin | Null cells | Max false acceptance | Its CP upper bound | Worst cell |
|---|---|---|---|---|
| 5 pp | 4,608 | 0.0060 | 0.0070 | 6%, RD 5, σ 2.5, asymmetric, 48 quartets |
| 7.5 pp | 3,456 | 0.0065 | 0.0075 | 6%, RD 7.5, σ 2.5, asymmetric, 48 quartets, linear drift |
| 10 pp | 2,304 | 0.0075 | 0.0086 | 6%, RD 10, σ 2.5, asymmetric, 48 quartets |

- **Coverage:** minimum 0.991 for U and 0.988 for L, with no cell's
  coverage upper bound below 0.95.
- **False worsening:** at RD ≤ 0, the maximum P(L > 0) is 0.005.
- **Error control:** C6 meets the V2 error-control and coverage
  criteria in every confirmed cell.
- **C2 as a reference:** it reproduced its failure at high precision
  (maximum false acceptance at δ = 7.5 of 0.169 at 8 quartets and 0.083
  at 48).

**Group G (invalidation).** In 11 cells at 48 quartets, almost no
experiment was valid (section 10), so the false-acceptance bound is wide
(Clopper–Pearson up to 0.063 from 0–61 valid experiments of 20,000).
Those cells are reported as **insufficient valid experiments**, not as
error control. At 8 quartets (5,382–14,575 valid experiments per cell),
C6's false acceptance under 5% and 10% invalidation is at most 0.0001.

**Why C6 is conservative, and its limits.** A block proportion lies in
[0, 1], so its variance is at most p(1 − p), even under full
within-block correlation. Treating each block as one unit therefore
cannot understate the variance of the mean, provided blocks are
independent. Two parts of this are not proven:

- the normal approximation of the score statistic with few blocks;
- the independence of blocks (they are fresh sandboxes, but a real
  host's drift could link them).

The confirmation covers drift and carry-over. **No coverage guarantee
is claimed beyond the simulated region.**

## 8. Power and sample size

C6 power at RD = 0, from the size sweep (equal seeds, no drift, no
asymmetry), δ = 7.5 pp (`results/power_C6_ni_75.csv`):

| p_A, σ | 4 Q | 8 Q | 16 Q | 24 Q | 32 Q | 48 Q | 64 Q |
|---|---|---|---|---|---|---|---|
| 2%, 0 / 1.5 | 0 / 0 | 0 / 0 | 0.03 / 0.08 | 0.69 / 0.65 | 0.99 / 0.94 | 1.00 / 1.00 | 1.00 / 1.00 |
| 4%, 0 / 1.5 | 0 / 0 | 0 / 0 | 0.01 / 0.08 | 0.26 / 0.36 | 0.74 / 0.67 | 1.00 / 0.95 | 1.00 / 0.99 |
| 6%, 0 / 1.5 | 0 / 0 | 0 / 0 | 0.00 / 0.07 | 0.10 / 0.23 | 0.40 / 0.46 | 0.92 / 0.79 | 1.00 / 0.94 |

**Confirmation group F at 48 quartets** gives C6's power at RD = 0 at
δ = 7.5 pp, as the minimum over asymmetry, carry-over, seeds and drift:

- **σ ≤ 1.5:** 0.997–1.000 at 2%, 0.919–0.991 at 4%, and 0.766–0.894 at
  6%;
- **σ 2.0–2.5:** 0.981–0.990 at 2%, 0.865–0.888 at 4%, and 0.719–0.739
  at 6%.

**Smallest simulated size with at least 80% power** for C6, over σ from
0 to 1.5 (Q = quartets; one block takes 19–21 minutes, measured in
Stage36 and Stage37):

| Margin | p_A = 2% | p_A = 4% | p_A = 6% |
|---|---|---|---|
| 5 pp | 48 Q (power at σ 1.5: 0.80) | > 64 Q | > 64 Q |
| 7.5 pp | 32 Q | 48 Q | 64 Q (48 Q: 0.77–0.79) |
| 10 pp | 24 Q | 24 Q | 32 Q |

What these sizes mean in practice:

| Size | Blocks | S3-P2 runs | Campaign time |
|---|---|---|---|
| 24 Q | 96 | 1,440 (720 legs per arm) | 30–34 h |
| 32 Q | 128 | 1,920 | 41–45 h |
| 48 Q | 192 | 2,880 | 61–67 h |
| 64 Q | 256 | 3,840 | 81–90 h |

The 120 runs per arm assumed in the survey (4 quartets) give C6 **no**
power, and every method that has power at that size fails error
control. Power above 64 quartets, and power at 64 quartets under the
nuisance factors, were not confirmed. Sizes between the simulated ones
(for example 40) were not simulated.

## 9. Sparse events

- **Experiments with no E1 event:**
  - at 0.5% and 4 quartets, 30% (σ = 0) and 34% (σ = 1.5);
  - at 1% and 8 quartets, 0.8–2.0%;
  - in group A, the maximum was 29% (2%, RD −1 pp, σ 1.5, 2 quartets).
- **Their treatment:** every such experiment is DEGENERATE under every
  method and gives no finding.
- **One arm with no events** is common (up to 60% at 0.5%). Every
  method computes ordinarily there; C6 then simply has no power.
- **No method gave false acceptance through a degenerate path:** the
  degenerate rule is applied before classification, and the tests check
  it.

## 10. Invalid quartets and replacement

Per-block invalidation probability b, with whole-quartet replacement
capped at 2. The probability that an experiment is valid, from an exact
calculation, matches the simulation (48 Q at 5%: 0.0027 calculated
against 0.0025 simulated; 8 Q at 5%: 0.7211 against 0.7208):

| Quartets | b = 0.1% | 0.2% | 0.5% | 1% | 5% |
|---|---|---|---|---|---|
| 8 | 1.000 | 1.000 | 0.999 | 0.994 | 0.721 |
| 24 | 1.000 | 0.999 | 0.986 | 0.919 | 0.115 |
| 32 | 1.000 | 0.998 | 0.970 | 0.851 | 0.035 |
| 48 | 0.999 | 0.993 | 0.923 | 0.685 | 0.003 |
| 64 | 0.998 | 0.984 | 0.856 | 0.515 | 0.000 |

- **When valid:** replacement does not degrade C6's error control. False
  acceptance under invalidation is at most 0.0001 at 8 quartets in
  confirmation, and 0 in screening group C. Replacement quartets run later, so
  drift interacts with them; the confirmation includes linear drift in
  every invalidation cell.
- **The practical constraint:** the size C6 needs is compatible with the
  two-quartet cap only if invalidation is very rare (about 0.2% of
  blocks or fewer for 99% validity at 48 quartets). For comparison,
  Stage36 had one host suspend in its eight blocks. Invalid
  experiments are reported as INVALID, never as a statistical result.

## 11. Interim monitoring (simulated only; not approved, V4)

One harm-only look at 24 of 48 quartets, rejecting on L > 0 at one-sided
α 0.005 or 0.001, with no early acceptance (`results/interim.csv`):

| Method, α | Stops at RD = 0 (false alarm) | Stops at RD = 7.5 | 10 | 15 | Effect on final acceptance at RD = 0 |
|---|---|---|---|---|---|
| C6, 0.005 | 0 | ≤ 0.001 | ≤ 0.020 | 0.005–0.38 | none (0.943 with and without) |
| C6, 0.001 | 0 | 0 | ≤ 0.0001 | ≤ 0.031 | none |
| C2, 0.005 | 0.003–0.006 | 0.44–1.00 | 0.70–1.00 | 0.95–1.00 | −0.4 pp |
| C2, 0.001 | ≤ 0.0014 | 0.22–1.00 | 0.46–1.00 | 0.86–1.00 | −0.05 pp |

- **With C6** the look is nearly useless: it misses a +15 pp regression
  in most runs.
- **With C2** it detects harm well, but C2's lower bound is itself not
  validated (coverage of L as low as 0.89 under dependence), so its
  false-alarm rate is established only in these cells.
- **Final false acceptance:** a harm-only look can only stop a trial,
  so it never increases final false acceptance.
- **Status:** no interim rule is recommended for live use.

## 12. Numerical stability

- **Failures:** C7 had 16 grid-edge FAILED replicates in screening, and
  no other method failed.
- **Degenerate replicates:** all methods share the zero/all-event count
  (29,398 in screening). C2 has 288 more, where the variance is zero
  with events. C3 has 67,867, mostly at 2 quartets, where the quartet
  differences are equal.
- **Not applicable:** C5 was not computed above 924 patterns (none in
  the grids run).
- **No replicate was regenerated.**

## 13. Rejected and qualified methods

| Method | Status | Reason |
|---|---|---|
| C0 | rejected | false acceptance up to 0.33; coverage down to 0.67 (diagnostic reference, as expected) |
| C1 | rejected | the estimated design effect under-inflates: false acceptance up to 0.23 |
| C2 | rejected | false acceptance up to 0.25 (screening) and 0.17 (confirmation); skewed block proportions |
| C3 | rejected | false acceptance up to 0.21; few degrees of freedom do not fix skewness |
| C4 | rejected | false acceptance up to 0.26; CR2 does not fix skewness |
| C5 | rejected (diagnostic) | no rejections at ≤ 4 quartets; false acceptance up to 0.145 at 6–8 quartets |
| C6 | **error control and coverage met; not certification-ready** | needs about 48–64 quartets (61–90 h) at δ = 7.5 pp, and a per-block invalidation rate near 0.2% |
| C7 | rejected | false acceptance up to 0.21; misspecified; grid-edge failures |

## 14. Remaining uncertainties

- **The model:** all results are conditional on the generative model.
  Real block dependence may not be logit-normal, and real invalidation
  may not be independent of time.
- **C6:**
  - **Theory:** C6's conservativeness has a partial argument (bounded
    block variance, independent blocks), not a proof, and the normal
    approximation with few blocks is unproven.
  - **Unconfirmed sizes:** C6 was not confirmed at 64 quartets under the
    nuisance factors, nor at sizes between the simulated ones.
- **E2** (the secondary endpoint) is generated and exclusive but was not
  analysed.
- **Methods not tried:** none outside C0–C7 was evaluated. A
  skewness-aware method (for example, an exact or bootstrap-t
  block-level procedure) might do better than C6, but that is new
  research, outside V7.

## 15. Recommendation for the next milestone

1. **Do not run the M4 A/A pilot yet.** It cannot help: no method is
   certification-ready at a practical size. The pilot was sized for a
   method like C2 or C4 at about 16 blocks, and those methods fail.
2. **An owner decision is needed** between:
   - **(a)** withholding comparative certification (the D4 fallback);
   - **(b)** accepting C6 with a multi-day campaign (48–64 quartets,
     61–90 h) and a far lower invalidation tolerance, or a larger
     replacement cap, which would be a V5 change;
   - **(c)** authorising research on a new, skewness-robust block-level
     method, with its own simulation.

   (a) or (c) is recommended. (b) is statistically defensible within
   the simulated region but operationally heavy, and its validity hinges
   on near-zero invalidation.
3. **tokio:** the tokio 1.54.1 question stays where Stage36 left it.
   Nothing here changes D4 or D5, or approves anything.

Not authorised by this report: the A/A pilot, a candidate trial, harness
changes, tokio promotion and release certification.
