# Stage38 M3b — frozen validation plan

Committed before any M3b estimator was implemented or run (M3b handoff
section 4.4). The qualification rules below do not change after results
are seen. Any later change is a recorded deviation, and the earlier rule
is still reported.

## 1. Hypotheses and findings

The estimand is RD = ⅓ Σ_s (p_B,s − p_A,s), per endpoint (E1: a deadline
event; E2: a clean early reset, excluding E1). One-sided α = 0.05.

- Non-inferiority: H0: RD ≥ δ against H1: RD < δ. Declared when U < δ.
- Inferiority: declared when L > δ. Worsening: declared when L > 0.
- Margins: E1 δ = 7.5 pp (provisional; sensitivity 5 and 10 pp); E2
  δ = 10 pp (provisional).

## 2. Qualification of a method (for each generator family it claims)

All rates are over valid experiments, with one-sided 95%
Clopper–Pearson Monte Carlo bounds. Any statement "rate ≤ x" means the
bound, not the point estimate.

1. **False acceptance.** At every required null cell (RD = δ, δ + 2.5 pp
   and δ + 7.5 pp where feasible), the Monte Carlo **upper** bound of
   P(U < δ) is at most 0.05.
2. **Coverage, three-way.** For each one-sided bound and each required
   cell (every RD in the grid), coverage is:
   - **adequate**: the Monte Carlo **lower** bound is at least 0.95;
   - **under**: the Monte Carlo **upper** bound is below 0.95;
   - **inconclusive**: otherwise.

   A method qualifies only if every required cell is *adequate*.
   Inconclusive cells are rerun with four times the replicates, once;
   if still inconclusive, the method is reported **inconclusive**, not
   qualified. (This corrects M3, whose rule accepted an upper bound of at
   least 0.95.)
3. **Degenerate cases.** Zero-event, one-sided-zero and all-event
   experiments have a defined, tested behaviour. A degenerate replicate
   never rejects. No substitute interval.
4. **Power.** At least 0.80 (point estimate) at RD = 0 for the E1
   margin, for baseline E1 rates 2%, 4% and 6% and block dependence up
   to the mandatory region (G1 σ ≤ 1.5).
5. **Practicality** (a provisional, pre-registered threshold for this
   report's classification; the owner decides finally):
   - **practical**: at most 16 quartets (64 blocks, about 21–22 h at
     19–21 min per block);
   - **impractical**: more than 16 quartets.

## 3. Required validation region

The required region per family, and each method's claim:

- **G1** (logit-normal blocks, M3): σ ∈ {0, 0.5, 1.0, 1.5}, with σ 2.0
  and 2.5 as sensitivity; asymmetric σ (+0.5 on B); carry-over φ ∈ {0,
  0.5}; seed heterogeneity; drift (none, linear ±, step).
- **G2** (mixture: occasional very bad blocks): a fraction π ∈ {0.05,
  0.15} of blocks with a logit shift of +3; marginal rates preserved.
- **G3** (dependent or nonstationary blocks): a shared host environment,
  AR(1) across consecutive blocks with ρ ∈ {0.5, 0.9}; and an abrupt
  deterioration (a logit step of +1 at a uniformly random campaign
  point). Both arms share the environment; treatment definitions are
  unchanged.

A method is reported as validated **for each family separately**. A
method valid in G1 only is not "validated" without that qualifier.

## 4. Stages

- **Phase A (mathematical):** a method whose assumptions contradict the
  design, or whose bound cannot reach δ at any data set at a given size,
  is rejected or limited with the argument written down.
- **Phase B (screening):** 2,000 replicates, root seed 38101. Screening
  rejects; it never qualifies.
- **Phase C (confirmation):** 20,000 replicates per cell, independent root
  seed 38102, for survivors and C6 (the benchmark). Inconclusive coverage
  is rerun at 80,000 replicates, root seed 38103.

No cell is removed after its result is seen. Every failing cell is kept.

## 5. E2 and the hierarchy

- **Procedure:** test E1 non-inferiority at α. Only if E1 is declared
  non-inferior, test E2 non-inferiority at α (fixed-sequence
  gatekeeping, family-wise error ≤ α).
- **Overall:** a comparative NONINFERIOR recommendation needs both
  endpoints declared non-inferior. A favourable E2 never compensates for
  E1. Failure to show E2 non-inferiority makes the overall result
  INCONCLUSIVE (or INFERIOR if L_E2 > δ_E2).
- **Worsening:** reported separately per endpoint, always.
- **Validation:** the family-wise rate of a false overall NONINFERIOR,
  in every cell where at least one endpoint's null is true, has Monte
  Carlo upper bound at most 0.05.

## 6. Replacement-policy study (design question only; V5 unchanged)

Policies: the V5 cap of 2 replacement quartets; a larger pre-registered
cap (6); immediate replacement (the replacement runs next); and restart
(declare invalid, rerun the whole experiment, at most 3 attempts).
Invalidation models: independent per block at 0.1%, 0.5%, 1% and 5%;
correlated within a quartet; driven by the host environment; and rising
late in the campaign. Measures: completion probability, blocks run,
false acceptance, power, and bias of the estimate.

## 7. Classification (handoff section 15)

- **M3B-A:** a new method qualifies (sections 2.1–2.4) in its claimed
  families **and** is practical (2.5).
- **M3B-B:** a method qualifies but is impractical.
- **M3B-C:** no method qualifies.
- **M3B-D:** the evidence cannot decide.
