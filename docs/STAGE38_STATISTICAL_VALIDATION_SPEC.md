# Stage38 — statistical validation specification

**Status: proposed validation contract for owner review.** This
document approves no method, margin or sample size. It authorises no
implementation, no pilot and no trial. None of its simulations has been
run. It applies the owner's Stage38 decisions
([survey, "Owner decisions"](STAGE38_COMPARATIVE_CERTIFICATION_SURVEY.md#owner-decisions-2026-11-02),
merged in #104, `73f54e9`):

- D1 is final;
- D4 is provisional;
- D5 is not approved;
- D3 is conditional on pilot evidence;
- D6 is conditional on simulation.

The baseline is `main` `73f54e927d4c138a61677bb76cd3028959bcc282`.
tokio is 1.52.3, quinn-proto 0.11.18 and S3-P2 is EXCLUDED; all three
are unchanged.

Labels used throughout:

- **Evidence** marks an assumption supported by Stage34–37 records.
- **Modelling choice** marks an assumption made here, which simulation
  must cover with a range, not a point.
- **Owner decision** marks a choice reserved for the owner.

## 1. Question

Can a prospective, finite-sample comparison show that a candidate's
S3-P2 TCP-hop liveness rate is non-inferior to a contemporaneous
control's, within an owner-approved margin, while controlling the
probability of falsely accepting non-inferiority?

The answer may be no for any practical sample size. In that case the
D4 consequence applies, and comparative certification is withheld.

## 2. Endpoints

All endpoints are defined over **valid TCP-hop legs** of the S3-P2
cell as `chaos.transfers` runs it, with the following unchanged:

- 512 KiB payload;
- 120 s bound;
- seed order 2010, 2020, 2030;
- QUIC leg first, then TCP leg;
- the Stage32 and Stage34 rules.

Raw outcomes are never changed (D1).

- **E1 (primary).** The leg reaches the 120 s deadline and is either:
  - raw `FAIL` with `failure_class` `liveness_bound`, or
  - raw `EXCLUDED` under the kernel-tail rule (`failure_signature`
    `kernel_tail`).
- **E2 (secondary).** The leg ends with an error before the deadline
  (a clean early end or reset), with no Layer A finding. E1 and E2 are
  mutually exclusive per leg.
- **E3 (descriptive).** Completion time of completed, integrity-matched
  legs.
- **Validity.** A leg, run or block is INVALID only for an
  independently established reason (section 9). Invalid legs enter no
  numerator and no denominator. A failure is never a validity reason.

QUIC-hop legs enter no Layer B statistic (D7). They are still checked
under Layer A.

## 3. Experimental structure and dependence

| Level | Structure | Basis |
|---|---|---|
| Leg | one TCP-hop leg per run | Evidence (harness) |
| Seed | 3 fixed netem seed positions; each block has 5 runs of each | Evidence (harness, Stage32 section 5) |
| Block | one sandbox, 15 consecutive runs, daemons and state persist | Evidence (harness; Stage34: fresh sandboxes do not reproduce cell behaviour) |
| Quartet | ABBA or BAAB; the arm is assigned per block | Design (D3) |
| Campaign | 16–32 blocks over 6–16 h on one host | Design |

**What the evidence supports about rates and dependence:**

- **Baseline E1 rate:** about 2–4%. The Stage34–37 pool on tokio
  1.52.3 has 6 E1 events in 150 runs. A tally of Stage36 build A and
  Stage37 alone gives 5 E1 events in 90 runs, in five sandboxes: 0, 1,
  1 and 2 events in the four 15-run Stage36 blocks, and 1 in the
  30-run Stage37 cell.
- **By seed** (the same 90 runs): 2010 1, 2020 1, 2030 3. This is
  consistent with equal seed rates, and also with seed 2030 being
  up to several times higher. The data cannot separate the two.
- **Block dependence:** not estimable from this evidence. With 5
  events in five sandboxes, any block-level intraclass correlation from 0 to
  well above 0.3 fits. Its potential mechanisms are known: state
  carried within the sandbox, persistent daemons, and the QUIC leg
  preceding the TCP leg. The size of their effect is not.
- **Time effects:** in Stage36, QUIC timing shifted between campaigns,
  and TCP medians varied 19–29 s between blocks of one build. A drift
  in E1 rate is plausible but unmeasured.

Everything else in the simulation model is a **modelling choice**.

## 4. Estimand and hypotheses

Let p_X,s be arm X's probability that a valid TCP-hop leg at seed
position s is an E1 event, averaged over the block-to-block
distribution (a marginal, population-averaged probability). The
estimand is the seed-standardised marginal risk difference:

  RD = (1/3) Σ_s (p_B,s − p_A,s)

A is the control and B the candidate. Seeds have equal weight, and the
design keeps them balanced in every block.

The hypothesis tests, for the approved margin δ (provisional 7.5 pp;
sensitivity at 5 and 10 pp):

| Finding | Null | Alternative | Reported when |
|---|---|---|---|
| Non-inferiority | H0: RD ≥ δ | H1: RD < δ | U < δ: `LIVENESS_NONINFERIOR` |
| Inferiority | H0′: RD ≤ δ | H1′: RD > δ | L > δ: `LIVENESS_INFERIOR` |
| Worsening | H0″: RD ≤ 0 | H1″: RD > 0 | L > 0: `WORSENING_DETECTED` |

U and L are the prespecified one-sided 95% bounds of the selected
primary method. When neither U < δ nor L > δ holds, the result is
`LIVENESS_INCONCLUSIVE`. A non-significant superiority or difference
test is never used in place of U < δ.

## 5. Candidate procedures (research candidates, not approved)

Each candidate yields (U, L) for RD.

| ID | Procedure | Respects clustering by | Known concerns to test |
|---|---|---|---|
| C0 | Seed-stratified Miettinen–Nurminen score interval, treating legs as independent | nothing | **diagnostic reference only**: expected to undercover under block dependence |
| C1 | C0 with a variance-inflation factor computed from the data (moment estimate of the block-level design effect, pooled over arms) | inflation | the owner's objections: an intraclass correlation estimate is not a design effect; the estimate is unstable at sparse counts; it may undercover |
| C2 | Cluster-summary analysis: per-block E1 proportion (seed-balanced by construction), arms compared by a Welch-type interval with small-sample degrees of freedom | blocks as units | zero-inflated, discrete block proportions; few blocks |
| C3 | Quartet-paired cluster summary: per quartet, (mean of B blocks) − (mean of A blocks); a t-interval over quartets | the randomised quartet structure | very few degrees of freedom (Q − 1); wide |
| C4 | Cluster-robust GEE: identity link (risk-difference scale), arm and seed terms, independence working correlation, with CR2 (Bell–McCaffrey) or Mancl–DeRouen corrections and Satterthwaite degrees of freedom | sandwich variance | non-convergence at the zero boundary; few clusters |
| C5 | Randomisation inference over the block-orientation assignment, inverted for H0: RD = δ under a constant-shift working model on the block-proportion scale | the actual randomisation | the attainable p-values are discrete (section 5.1); the shift model is an assumption |
| C6 | Worst-case clustering bound: C0 computed with an effective sample size equal to the number of blocks (full within-block correlation) | worst case | expected to be very conservative; serves as a lower bound on achievable power |
| C7 | Beta-binomial (block-level overdispersion) likelihood, with a profile-likelihood bound for RD | a parametric model | sensitivity to the model; sparse-data behaviour |

**Sparse and zero-event samples.** Every candidate must define its
output when one or both arms have 0 events. C2–C4 can degenerate
(zero variance), and the procedure must then fall back to a
prespecified conservative rule, for example C6, chosen before the
trial. An exact interval derived under independence is not a valid
fallback on its own (D5).

### 5.1 Granularity of randomisation inference

Read-only calculation:

| Quartets Q (blocks) | Free orientation per quartet: assignments, smallest p | Balanced orientations (survey D3): assignments, smallest p |
|---|---|---|
| 2 (8) | 4, 0.25 | 2, 0.5 |
| 4 (16) | 16, 0.0625 | 6, 0.167 |
| 6 (24) | 64, 0.016 | 20, 0.05 |
| 8 (32) | 256, 0.004 | 70, 0.014 |

With the survey's recommended 16 blocks, **C5 can never reach a
one-sided p of 0.05**. The balance constraint on orientations reduces
the assignment set further. If C5 is shortlisted, the owner must
choose between free orientations and at least 5 quartets (owner
decision V3).

## 6. Simulation design

### 6.1 Generative model (modelling choice)

For block b in arm X, at seed position s, run r, with b's campaign
position t_b:

  logit P(E1) = μ_X,s + τ·g(t_b) + u_b + w_b,r

where:

- **μ_X,s** sets the marginal rates. It is calibrated so that the
  seed-standardised marginal RD equals the scenario's true RD.
- **g(t)** is a time-drift shape: none, linear across the campaign, or
  a mid-campaign step.
- **u_b ~ N(0, σ_X²)** is the block effect. It may differ by arm
  (asymmetric dependence).
- **w_b,r** is within-block carry-over: an AR(1) latent effect over the
  run sequence, with persistence φ.

E2 events are generated jointly (a multinomial outcome per leg:
complete, E1 or E2, with its own baseline rate of about 12%, Evidence).
This lets E2 and the hierarchy be checked.

**Diagnostic comparison:** independent Bernoulli legs (σ = 0, φ = 0,
τ = 0). It is never the sole basis for a decision.

### 6.2 Parameter grid

| Parameter | Values |
|---|---|
| Baseline marginal E1 rate p_A | 2%, 4%, 6% (Evidence range) |
| True RD | −2, −1, 0, +2.5, +5, +7.5, +10, +15 pp (negative values only where p_A + RD ≥ 0) |
| Block SD σ (logit) | 0, 0.5, 1.0, 1.5 (latent intraclass correlation about 0, 0.07, 0.23, 0.41) |
| Asymmetric dependence | σ_B = σ_A, and σ_B = σ_A + 0.5 |
| Carry-over φ | 0, 0.5 |
| Seed heterogeneity | equal; seed 2030 at 3× the others (Evidence-compatible) |
| Time drift τ (logit, campaign-wide) | 0; linear ±0.5; step +0.5 at the midpoint |
| Blocks per arm (runs per arm) | 4 (60), 8 (120), 12 (180), 16 (240); 20 and 24 if needed |
| Orientation | balanced (survey D3); free per quartet |
| Margin δ | 7.5 pp (provisional); 5 and 10 pp (sensitivity) |
| Interim | none; one harm look at 50% (section 8) |
| Confidence | one-sided 95% |

The plausible nuisance region is σ up to 1.0, with either value of φ,
either seed pattern and any drift. Strong clustering (σ = 1.5) is a
stress region (owner decision V1).

### 6.3 Operating characteristics

For each procedure and grid cell:

- **False acceptance** (type I error for H0): P(U < δ | RD = δ), and at
  RD slightly above δ.
- **Coverage:** P(RD ≤ U) and P(RD ≥ L) at every RD.
- **Power:** P(U < δ) at RD = 0 and at the negative RDs.
- **Inferiority and worsening rates:** P(L > δ) and P(L > 0) at each
  RD, including the false-worsening rate at RD ≤ 0.
- **Degeneracy:** the frequency of zero-event and degenerate outcomes,
  and the fallback's share.
- **Bias** of the point estimate of RD.

### 6.4 Monte Carlo precision

| Quantity | Replicates per grid cell | Precision (from read-only calculation) |
|---|---|---|
| False acceptance and coverage at RD = δ | 20,000 | SE 0.0015 at 5%; one-sided 95% MC upper bound 0.0525 if the estimate is 0.05 |
| Power, inferiority and worsening rates | 5,000 | SE ≤ 0.007 at 0.8 |
| Screening runs (optional, before full runs) | 2,000 | SE 0.005 at 5%; screening only, never for acceptance |

Every reported rate carries its Clopper–Pearson 95% interval. Random
number generator seeds are fixed per cell and recorded.

### 6.5 Acceptance criteria for a primary method (proposed; owner decision V2)

A procedure may be proposed as the primary method only if all of the
following hold:

1. **False acceptance controlled:** at every cell of the plausible
   nuisance region with RD = δ, the false-acceptance estimate is at most
   0.05, and its one-sided 95% Monte Carlo upper bound is at most 0.055.
2. **Stress behaviour reported:** in the stress region, false
   acceptance is reported. It may exceed 0.05 only if the owner accepts
   that region as implausible, in writing, with the pilot's evidence.
3. **Coverage:** the upper bound U covers RD (RD ≤ U) with at least
   94% probability across the plausible region.
4. **Power at a practical size:** at least 80% at RD = 0, at the
   smallest practical whole number of quartets, for p_A = 4% and
   σ ≤ 0.5. Power under stronger dependence is reported.
5. **Defined degenerate cases:** zero-event and degenerate outcomes
   have a defined, prespecified result.

If no candidate meets criteria 1, 3 and 5, **no method is approved**,
and comparative certification is withheld (D4). If criteria 1, 3 and 5
are met but criterion 4 needs an impractical size, the result is the
same unless the owner widens δ, which is a separate decision.

### 6.6 Sample-size method

The sample size is the smallest whole number of quartets meeting
criterion 4 for the chosen method, with criteria 1, 3 and 5 confirmed
at that size. Planning figures under independence, which are illustrative
only and anti-conservative, are in survey section 7. Each block takes
about 20–30 minutes. 16 blocks are about 6–8 h, and 32 blocks about
12–16 h, which needs suspend inhibition for the whole campaign.

## 7. A/A pilot (designed, not authorised)

**Purpose.** The pilot has three aims:

- verify the block runner, the records and the validity checks
  end-to-end;
- measure the environment: the E1 and E2 rates, block-to-block spread
  and drift;
- detect any unexpected asymmetry between two labels of the same
  build.

It is **not** expected to estimate a rare-event intraclass correlation
precisely, and the primary method must be valid across the grid, not
tuned to a pilot estimate.

**Design:**

- **Builds:** both arms use the byte-identical production binaries of
  `main`, with SHA-256 recorded and checked before every block. The
  labels A and A′ are assigned by the trial's randomisation procedure.
- **Size:** 16 blocks (4 quartets, 240 runs), the same orientation
  procedure as the trial (free or balanced, per V3), 15-run blocks and
  the established seed order.
- **Host controls:**
  - one campaign at a time, with no builds and no other sandboxes;
  - the repository untouched;
  - the kernel version and the relevant `net.*` sysctls recorded and
    unchanged;
  - load average per run.
- **Suspend:** a `systemd-inhibit` lock for the campaign, plus per-run
  detection: an increase of more than 1 s in `CLOCK_BOOTTIME −
  CLOCK_MONOTONIC` from leg start to leg end invalidates the block.
- **Reproducibility:** the D8 evidence package (section 11), the
  preregistered orientation seed, and the analysis code version.

**Informative-pilot criteria:**

- at least 14 valid blocks;
- no validity failure that the runner could not detect;
- the E1 rate estimated, with its interval;
- an overdispersion check on block E1 counts, reported, using a
  prespecified score test of the beta-binomial against the binomial;
- the A/A comparison under every shortlisted procedure, reported. A
  `WORSENING_DETECTED` result between identical builds is expected
  about 5% of the time. A more extreme result, or several procedures
  agreeing on one, triggers a review of the runner and environment
  before any candidate trial.

**If no E1 events occur:**

- the rate is plausibly below about 1.2% (3 events in 240 at the upper
  95% bound);
- dependence cannot be assessed;
- the trial then relies on procedures valid over the full grid,
  including worst-case behaviour (C6);
- the owner should reconsider whether E1's provisional margin is
  meaningful at such a rate (V6).

## 8. Interim monitoring and stops

The stop categories are kept separate. Each stop is recorded with its
category.

| Category | Trigger | Outcome |
|---|---|---|
| S1, absolute | any Layer A violation in either arm: integrity, order, crash or panic, leak beyond the Stage32 bounds, downgrade, fence or authorization | immediate stop. Candidate: Layer A `VIOLATED`. Control: the trial is invalid and a defect is opened (Stage32 section 7). |
| S2, validity | more than the allowed replacements (section 9), a build checksum mismatch, a host or kernel change, or loss of records | stop; trial invalid; nothing concluded |
| S3, diagnostic hold | a genuinely new candidate-only failure kind (D1) | stop; Layer A `HELD`; never resumed; any new trial needs a new preregistration |
| S4, deterioration signal | at the planned interim look (50% of quartets), the interim lower bound exceeds 0 at a stringent level (candidate boundaries: one-sided α 0.001 or 0.005, to be chosen by simulation) | stop for investigation; recorded as `WORSENING_DETECTED (interim)`. **This is not** `LIVENESS_INFERIOR`. |
| S5, inferiority at interim | the interim lower bound exceeds δ at the S4 level | stop; `LIVENESS_INFERIOR` only if this boundary was prespecified and simulated |
| — | everything else | continue to the preregistered size; **no early acceptance** |

The simulation includes the interim look. It reports:

- its effect on the final false-acceptance rate (expected to be at most
  the no-interim rate, because the look only stops);
- its effect on power;
- the probability of an S4 stop at RD = 0, a false alarm.

The interim boundary is chosen from those results (V4). Early
acceptance would need a separately reviewed sequential procedure.

## 9. Invalid blocks and replacement

**Objective, preregistered validity reasons only.** Each is decided by
a script that reads run metadata, not E1 outcomes. Its verdict is
logged before any comparative analysis:

1. a detected suspend in any run of the block (section 7);
2. a binary checksum differing from the preregistered arm checksum;
3. a demonstrable harness failure: a harness exception, or a harness
   `EXCLUDED` reason other than `kernel_tail`;
4. a host-load breach above a preregistered threshold;
5. missing records.

A genuine transport timeout, reset or kernel tail is never a validity
reason.

**Replacement preserves the design:**

- **Whole quartet.** An invalid block makes its whole quartet
  incomplete. A complete replacement quartet, with a newly randomised
  orientation, is appended at the end of the campaign.
- **Valid blocks of an incomplete quartet** stay in the record. They
  are excluded from the primary analysis, because their exclusion
  depends only on another block's validity, and are included in a
  sensitivity analysis.
- **Cap.** At most 2 replacement quartets per trial. Beyond that is a
  validity stop (S2).
- **Effect on inference.** The primary analysis uses complete quartets
  only, so the randomisation structure that C3 and C5 rely on is
  intact. Replacement quartets run later, and drift is handled by the
  model checks in section 10. The simulation includes an invalidation
  rate of 0%, 5% and 10% of blocks, with replacement, to measure the
  effect on error rates.
- **Records.** All original observations, including every invalid
  block, stay in the evidence record with their reasons.

## 10. Sensitivity analyses

These are prespecified and reported. They never override the primary
method.

- every other shortlisted procedure;
- including the valid blocks of incomplete quartets;
- including invalid blocks;
- excluding the first block of each arm;
- E1 split into raw FAIL and kernel tail;
- per-seed estimates;
- drift checks: E1 against campaign position, and an
  arm-by-position interaction;
- the C6 worst-case bound.

Where a sensitivity analysis disagrees with the primary on any finding,
the disagreement is stated in the record, and the owner decides. The
primary finding is not replaced.

## 11. Machine-readable evidence

| Artefact | Content |
|---|---|
| `prereg.json` (docs-only commit before the first block) | estimand, endpoints, δ, α, primary method and fallback, sensitivity list, quartet count, orientation procedure and seed, interim boundary, validity thresholds, replacement cap, arm checksums, analysis code commit |
| `runs.csv` (SHA-256 in the record) | arm, block, quartet, position, seed, run, wall start and end, boot−monotonic delta, load, `run_outcome`, `failure_class`, `failure_signature`, `provenance`, E1, E2, E3, validity and reason, both legs' bytes, hashes and times |
| `validity.log` | the validity script's verdicts, written before analysis |
| `analysis.json` | for the primary and each sensitivity: U, L, point estimate, the findings, degeneracy flags, code commit |
| `simulation/` (offline) | per grid cell: procedure, parameters, replicates, generator seed, false acceptance, coverage, power, worsening and inferiority rates, each with a Clopper–Pearson interval; code commit |
| record (`docs/`) | every table reproduced from `runs.csv` and `analysis.json` alone |

Binaries, packet captures and large logs stay under `target/`, outside
Git.

## 12. Conditions that prevent certification

Any one of the following prevents a `LIVENESS_NONINFERIOR` result:

- D4 margins not final;
- no D5 primary method approved under section 6.5;
- no preregistration, or a deviation from it that affects the primary
  analysis;
- Layer A `HELD`, `VIOLATED` or `INCOMPLETE`;
- a validity stop (S2), or fewer complete quartets than preregistered;
- the primary method's fallback triggered where its fallback was not
  prespecified;
- missing evidence artefacts.

`LIVENESS_INCONCLUSIVE` is a valid result, and it is never reported as
non-inferiority.

## 13. Decision table for owner review

| # | Decision | Options | Proposed |
|---|---|---|---|
| V1 | Plausible nuisance region | σ ≤ 0.5 · σ ≤ 1.0 · σ ≤ 1.5 | σ ≤ 1.0; σ = 1.5 as the stress region |
| V2 | Primary-method acceptance criteria | section 6.5 as proposed · stricter (Monte Carlo upper bound ≤ 0.0525) · looser | as proposed |
| V3 | Orientation randomisation | balanced (survey D3) · free per quartet | free, if C5 is shortlisted; otherwise balanced |
| V4 | Interim look | none · one harm look at 50%, boundary from simulation | one harm look, boundary chosen from the simulated false-alarm rate |
| V5 | Replacement policy | whole quartet, cap 2 (section 9) · single block, matched arm | whole quartet, cap 2 |
| V6 | Action if the pilot sees no E1 events | proceed on full-grid validity · reconsider E1 and δ | owner review before any trial |
| V7 | Candidate shortlist for the simulation | C0–C7, or a subset | all; C0 as a diagnostic only |
| V8 | Monte Carlo replicates | section 6.4 · fewer (screening only) | section 6.4 |
| V9 | Next milestone (each needs its own authorisation) | simulation code and run (M3) · A/A pilot (M4) | M3 first, offline only; the pilot after the simulation has narrowed the shortlist |

Not authorised by this document: implementation, the simulation run,
the A/A pilot, a candidate trial, tokio promotion and release
certification.
