# Stage38 — comparative transport certification: survey

**Status: survey for owner review.** It sets out decisions D1–D9 with
options and recommendations. Nothing here is approved, implemented or
in force until the owner accepts it. Stage32, Stage34 and Stage37 rules,
`chaos.py`, the kernel-tail classifier, transport settings and
dependencies are unchanged. tokio stays at **1.52.3** on `main`, with
RUSTSEC-2026-0386 accepted. S3-P2 stays **EXCLUDED**. The frozen
`v1.0.0-rc.2` tag and archive are untouched. No historical verdict is
rewritten.

Baseline: `main` `c5400aedf61cc3eeef3228d9503460b33ae424e0`, after
PR #102.

In this document:

- **Fact** marks something established by a merged record, with the
  record named.
- **Recommendation** marks this survey's proposal.
- **Decision** marks what the owner must choose. All numbers derived
  here come from existing evidence by small read-only calculations.
  No new campaign was run.

## 1. Executive summary

The proposal is two independent layers, judged separately and never
merged into one verdict.

- **Layer A, absolute:** every security, correctness, integrity, crash,
  leak and resource requirement that exists today, unchanged. A single
  violation rejects the candidate, whatever the baseline does.
- **Layer B, comparative transport liveness:** this applies only to a
  limitation that is already documented and that the owner has listed
  as comparison-eligible; today the only such limitation is the S3-P2
  encrypted-TCP hop. It asks one question: is the candidate's liveness
  failure rate non-inferior to a contemporaneous baseline within a
  pre-registered margin?

The answer is `LIVENESS_NONINFERIOR`, `LIVENESS_INFERIOR` or
`LIVENESS_INCONCLUSIVE`. It is never PASS. Every run keeps its raw
Stage32 outcome (PASS, FAIL or EXCLUDED). A FAIL stays a FAIL for the
baseline and for the candidate.

Recommended method:

- **Design:** a prospective, pre-registered, counterbalanced block
  design (ABBA and BAAB quartets of whole S3-P2 cell-sequence blocks).
- **Endpoint:** one primary endpoint, the TCP-hop liveness-failure
  rate (legs at the bound, kernel tails included).
- **Test:** a one-sided 95% upper confidence bound on the
  seed-stratified risk difference, confirmed by a block-level bootstrap.
- **Margin:** recommended at 7.5 percentage points, for the owner to
  approve.
- **Size:** about 120 runs per arm, in 16 blocks (Recommendation, from
  the sizing method in section 7).
- **Pilot:** the same design run baseline against baseline (an A/A
  pilot) before any candidate trial, to measure block dependence and
  confirm that the procedure does not produce false differences.

Applied to the worked case (section 11): Stage35's absolute gate would
have stopped the unchanged baseline itself in about 45–71% of 30-run
trials. Stage36 is strong retrospective evidence against a material
regression, but it cannot be counted as a non-inferiority result,
because it had no pre-registered margin. tokio 1.54.1 therefore needs a
prospective Layer B trial, and the Layer A gates Stage35 did not reach
(the Stage32 QUIC subset), before any integration decision.

## 2. Motivation and factual background

**Fact (Stage32 spec section 4):** a run is PASS only if every
applicable item holds: integrity, the bounded-liveness outer bound,
no downgrade, fences, no crash or leak, and the control plane.
Anything the harness cannot establish is EXCLUDED, never PASS. A
scenario passes only if all of its runs pass.

**Fact (Stage34 record; spec "kernel-tail rule"):** for S3-P2:

- the payload is 512 KiB;
- the bound is 120 s;
- a routed TCP-hop leg that reaches the bound after Y has relayed
  everything and closed its socket is EXCLUDED as a kernel tail;
- any other stall is FAIL;
- certification is 30 runs with no FAIL and the tail count stated;
- S3-P2 is EXCLUDED because of the quinn-proto 0.11.18 regression of
  the routed QUIC hop, which is still present in 0.11.19.

**Fact (Stage33 QUIC/TLS remediation):** tokio 1.53.3 was withdrawn
after a real regression. Six of its 18 S3-P2 TCP-hop legs failed (five
at the bound, still progressing; one reset at 119.7 s), against 0 of 8
and 0 of 14 on tokio 1.52.3 builds.
RUSTSEC-2026-0386 (unsound `AsyncFd`, fixed in ≥ 1.53.3) was accepted:
neither Werewolf nor any shipped dependency other than tokio uses
`AsyncFd`.

**Fact (Stage35):** tokio 1.54.1 passed advisories (0 vulnerabilities,
0 warnings), fmt, clippy, 412 Rust tests, 64 Python unit tests, 12
acceptance tests, ten lab gates and the Stage12 proofs. The S3-P2 gate
stopped the trial, with run 28 a `CLOSE-WAIT` FAIL and two kernel tails
in 30 runs. The Stage32 QUIC subset was not run. The verdict was
REJECTED.

**Fact (Stage36):** in a 120-run ABBA and BAAB comparison of identical
source, the TCP and QUIC results showed no material regression. The
`CLOSE-WAIT`-at-bound FAIL reproduced on tokio 1.52.3 (2 in 30
uninstrumented runs). tokio 1.54.1 was accepted for further
certification only.

**Fact (Stage37):** with lifecycle tracing on tokio 1.52.3, the stall is
correct application behaviour under kernel backpressure (Outcome A):

- the relay waits in one frame write on a socket that is never
  writable;
- retransmissions are lost under exponential backoff;
- `TCP_USER_TIMEOUT` (60 s) ends the connection;
- no application defect and no missed wakeup were found.

**Pooled observed rates** at 512 KiB, S3-P2 cell sequence, TCP hop
(read-only arithmetic over the records; Wilson 95% intervals):

| | Runs | At the bound (FAIL + kernel tail) | Of which FAIL (non-tail) | Clean early ends |
|---|---|---|---|---|
| tokio 1.52.3 (Stage34 ×2, Stage36 A, Stage37 traced) | 150 | 6 (4.0%, 1.8–8.5%) | 3 (2.0%, 0.7–5.7%) | 11 of 90 recorded (12%, 7–21%) |
| tokio 1.54.1 (Stage35, Stage36 B) | 90 | 4 (4.4%, 1.7–10.9%) | 1 (1.1%, 0.2–6.0%) | |
| tokio 1.53.3 (Stage33 withdrawn) | 18 | 6 liveness failures, 5 at the bound (33%, 16–56%) | | |

These pooled figures mix campaigns, days, instrumentation states and one
traced binary. They are for planning and illustration only, never for
a decision.

**The problem.** With a baseline liveness-failure rate of roughly 2–4%,
an absolute "30 runs, no FAIL" gate:

- stops an unchanged build in 45% (p = 2%) to 71% (p = 4%) of trials;
- therefore cannot discriminate a harmless candidate from the baseline;
- yet would still catch a regression like 1.53.3's.

The opposite risk is equal: "the baseline fails too" must never become a
reason to accept.

## 3. Two-layer architecture

```text
candidate change ──► Layer A (absolute) ──► any violation ──► REJECT (no comparison can rescue)
                         │ all satisfied
                         ▼
                 Layer B (comparative, only for listed limitations)
                         │
           NONINFERIOR / INFERIOR / INCONCLUSIVE   (raw PASS/FAIL/EXCLUDED kept per run)
                         │
                         ▼
          remediation recommendation ──► owner integration decision ──► separate release certification
```

**Layer A (absolute; Recommendation: unchanged from today).** These are
hard requirements, with no statistical allowance:

- authentication, peer identity, Target Policy authorization, Silver
  and governance enforcement, and replay protection;
- wire compatibility and protected-state invariants;
- data integrity: every completed transfer, and every echoed prefix,
  byte-exact and in order;
- no downgrade to plain TCP;
- fences;
- no crash or panic;
- descriptor, thread and RSS bounds after settle (Stage32 item 5);
- control-plane exactness;
- advisory status of the candidate tree;
- every existing test suite and lab gate;
- every Stage32 cell **not** listed as comparison-eligible, under its
  existing rule. A FAIL in a non-eligible cell is a Layer A failure.

**Layer B (comparative).** It applies to a cell-leg only when all three
of these hold:

1. a merged record documents the limitation and its mechanism;
2. the limitation is in `KNOWN_LIMITATIONS.md`;
3. the owner has listed it as comparison-eligible.

Recommendation: the initial list is one entry, the **S3-P2 encrypted-TCP
hop** (Stage34 kernel tail and the Stage37 `CLOSE-WAIT` backpressure).
Layer B measures and reports these. It never reclassifies runs and never
creates a PASS.

## 4. Decisions D1–D9: options

### D1 — Terminology

- **1a (Recommended): separate machine-readable fields.** Each field has
  a closed vocabulary (section 5).
- **1b:** keep today's PASS, FAIL and EXCLUDED, and put comparisons in
  prose only. Rejected: not machine-checkable, and it invites
  conflation.
- **1c:** introduce a "comparative PASS". Rejected: it renames failure
  as success, which violates invariant 10 and the governing principle.

### D2 — Reference baseline

- **2a:** the current production source and lockfile, built fresh as a
  contemporaneous control.
- **2b:** the last certified release artifact (rc.2).
- **2c:** both.
- **2d (Recommended):** 2a is the control for every analysis.
  - The rc.2 artifact is used as a historical anchor only: its checksum
    is recorded, and equality with the control is stated when it holds.
    Today the control `werewolfd` would equal the rc.2 certification
    build, `fe66ba76…05c5` (Stage36).
  - When `main` source differs from rc.2, the rc.2 artifact is never
    substituted for the control.

### D3 — Experimental design

- **3a:** alternating single runs. Rejected: one sandbox runs a whole
  cell sequence, and daemon state persists between runs. Single-run
  alternation is not possible without changing the harness's sequence.
- **3b:** two long contiguous campaigns, A then B. Rejected: confounded
  with time of day and host state (Stage36 saw QUIC timing shift
  between campaigns).
- **3c (Recommended):** whole-block counterbalancing.
  - **Block:** one fresh sandbox running `chaos.transfers` for S3-P2, 5
    rounds of the established seed order 2010, 2020, 2030: 15 runs,
    each a QUIC-hop leg then a TCP-hop leg, exactly as the harness
    defines.
  - **Order:** blocks are arranged in quartets, each ABBA or BAAB. Each
    quartet's orientation is drawn by a pre-registered random seed,
    recorded before the first block, with the constraint that the
    whole sequence has equal numbers of each orientation.
  - **Size:** 8 blocks per arm (16 blocks, 120 runs per arm), as sized
    in section 7. Changeable only before the trial starts.
  - **Host controls:**
    - one campaign at a time;
    - no builds or other sandboxes while running;
    - the repository untouched;
    - a suspend inhibitor held for the campaign (`systemd-inhibit`);
    - per run: load average, the CPU frequency governor and the clock
      readings.
  - **Suspend and resume detection:** per run, the difference between
    `CLOCK_BOOTTIME` and `CLOCK_MONOTONIC` at leg start and leg end. Any
    increase of more than 1 s marks the run, and its block, INVALID
    (section 6.6).
  - **Instrumentation:** certification blocks run uninstrumented, except
    the harness's own per-second Y-socket sampling, which the kernel-tail
    rule requires. Packet capture and tracing are allowed only in
    separately labelled diagnostic campaigns, never pooled with
    certification blocks. Stage36 found no measurable TCP effect of
    capture, but a borderline QUIC effect, so the arms must not differ
    in instrumentation.
  - **Configuration records** are in D8.
- **3d:** adaptive designs that re-randomise mid-trial. Not recommended
  without a separate statistical review.

### D4 — Non-inferiority margins

The margin δ is the largest increase in a failure rate, candidate minus
baseline, that the owner accepts as not material. Candidate margins are
below, all on the absolute risk difference. A ratio margin is unstable
at rates near 2–4%, because one event changes it greatly.

| Endpoint | Options for δ | Recommendation | Rationale and consequence |
|---|---|---|---|
| **E1 (primary): TCP-hop liveness failure** (leg at the bound: FAIL plus kernel tail) | 5, 7.5, 10 pp | **7.5 pp**, owner to approve | See below |
| E2: TCP-hop clean early end (setup or session reset, within the bound) | 10, 15 pp | 10 pp, **secondary** | Baseline about 12% (7–21%). Resets are a visible, bounded failure, not a hang, so a wider margin is tolerable. |
| E3: TCP-hop completion probability (complete and integrity-matched) | its complement is the union of E1, E2 and other errors | reported; no separate margin | Avoids double-counting E1 and E2 |
| E4: TCP-hop transfer latency of completed legs | ratio of medians ≤ 1.25 or ≤ 1.5 | **descriptive only** until a pilot exists | Between-campaign medians on the same build varied 19–29 s (Stage36), so a defensible margin needs the A/A pilot's block variance |
| E5: establishment latency (time to first echo; S4 cells for opens) | none yet | **descriptive only** | No baseline distribution recorded for S3-P2 TCP; S4 opens are Layer A (absolute) |

**The E1 margin.**

- **What 7.5 pp means:** from a baseline near 4%, accepting a candidate
  whose true rate could be as high as about 11.5%, roughly one extra
  stalled routed session in 13 under this extreme profile.
- **What it detects:** a regression of the 1.53.3 kind (33%) by a wide
  margin, and anything else above about 11.5%.
- **What it would accept:** a smaller real increase, of up to 7.5 pp.
- **Sensitivity:**
  - δ = 5 pp halves the tolerated increase but needs about 190–265 runs
    per arm (section 7);
  - δ = 10 pp needs about 50–70 runs per arm, but would accept a rate
    up to about 14%.
- **Operating consequence:** the profile exceeds any normal deployment.
  An S3-P2 stall ends in a bounded, reset-signalled failure, never a
  silent truncation (Stage37 section 12). A modest increase is an
  operational, not a security, cost.
- **Not a statistical finding:** the evidence does not determine a
  "right" margin. The choice is the owner's risk tolerance. If the owner
  cannot accept any of these margins, the Recommendation is **4d
  below**.

Options:

- **4a:** E1 primary at 7.5 pp; E2 secondary at 10 pp; E3–E5
  descriptive (Recommended).
- **4b:** E1 at 5 pp, with a larger sample.
- **4c:** E1 at 10 pp.
- **4d:** withhold Layer B certification until a margin is approved.
  Candidate trials then report estimates only, and no
  `LIVENESS_NONINFERIOR` can be issued.

### D5 — Statistical method

- **5a (Recommended):**
  - **Primary analysis:** the one-sided 95% upper confidence bound of
    the risk difference (candidate − baseline) for E1, stratified by
    seed, using the Miettinen–Nurminen stratified score interval with
    Cochran–Mantel–Haenszel weights.
  - **Robustness analysis:** a block-cluster bootstrap. Blocks are
    resampled within arm, keeping each block whole, with 10,000
    replicates and a percentile upper bound.
  - **The decision uses the larger (more conservative) of the two
    upper bounds.**
  - Details in section 6.
- **5b:** an unstratified Fisher test or p-value. Rejected: a
  non-significant p-value is not non-inferiority, and it ignores seeds
  and blocks.
- **5c:** a Bayesian beta-binomial with a pre-registered prior and a
  posterior probability threshold. A reasonable alternative, but harder
  to audit for a private project with one owner. Not recommended now.

### D6 — Sample size and stopping

- **6a (Recommended):**
  - a fixed sample, prospectively sized by simulation (section 7);
  - hard stops for Layer A and for validity;
  - one pre-registered interim look for harm only, at the midpoint
    (8 blocks);
  - **no early stop for acceptance.**
- **6b:** a full group-sequential design with alpha spending for both
  acceptance and harm. Possible later, but it needs a statistical
  review and an implemented spending function. Not for the first trial.
- **6c:** run until the result looks acceptable. Rejected: optional
  stopping invalidates the confidence bound.

### D7 — S3-P2 while the QUIC hop is excluded

- **7a (Recommended):** the legs are separated by construction.
  - **TCP-hop legs** feed Layer B, E1 and E2.
  - **QUIC-hop legs** are still run, because the cell sequence must stay
    identical, and still checked under Layer A: integrity, crashes,
    leaks and downgrades are absolute. Their liveness is recorded and
    reported but excluded from every Layer B statistic and decision.
  - **The QUIC exclusion is unchanged:** S3-P2 stays EXCLUDED. A QUIC
    leg failure can neither excuse nor offset a TCP leg result.
  - **Labelling:** Layer B output is labelled
    "S3-P2 TCP-hop comparative liveness", never "S3-P2 certified".
  - **quinn changes:** a quinn or quinn-proto change is out of Layer B's
    S3-P2 scope. It must meet the Stage34 retest conditions (the
    regression check on seed 2030, then 30 runs with no FAIL).
- **7b:** pause all S3-P2 use until QUIC is fixed. Rejected: it would
  leave the TCP limitation unmeasurable for unrelated dependency
  changes, such as tokio.

### D8 — Evidence and auditability

- **8a (Recommended):** a pre-registration file, committed (docs-only)
  before the first block. It records the trial's endpoints, margins,
  α, sample size, block sequence seed, stop rules and analysis code
  version.
  - **Evidence package:** the trial's evidence package, kept under
    `target/`, holds every item listed in section 10.
  - **Record:** a docs-only record reproduces every table from a
    per-run CSV whose SHA-256 is stated.
  - **Late changes:** any change after the first block is a protocol
    deviation, and it is reported as such.
- **8b:** a record written after the fact, as in Stage36. Acceptable
  for diagnosis, not for Layer B certification.

### D9 — Advisory remediation and release policy

Five distinct statuses are recorded independently. None implies the
next.

1. `advisory_resolved`: the candidate tree is free of the advisory
   (`cargo audit`, database commit stated).
2. `layer_a_status`: `SATISFIED` when every absolute gate has passed,
   including those not yet run.
3. `liveness_comparison`: the Layer B result for each eligible
   limitation.
4. `integration_decision`: the owner's architectural decision to merge
   into `main`.
5. `release_certification`: a later, separate certification of a
   release candidate (archive, gates, audit). The comparison feeds into
   it but never substitutes for it.

- **9a (Recommended):** all five are required in order. A
  `LIVENESS_INCONCLUSIVE` result leaves integration to the owner, who
  may defer, run more blocks under a new pre-registration, or accept
  the risk explicitly in writing. It is never recorded as
  non-inferiority.
- **9b:** integrate on 1 and 2 alone, for security fixes of
  high severity. Recommended as a documented **emergency path** only.
  It needs an explicit owner decision naming the advisory and the
  unmeasured liveness risk, and it creates a follow-up Layer B trial
  obligation.

Applied to tokio 1.54.1 and RUSTSEC-2026-0386 (today):

| Status | Value | Basis |
|---|---|---|
| advisory_resolved | yes, for that candidate and database (`2295466c`) | Stage35 |
| layer_a_status | **INCOMPLETE** | the Stage32 QUIC subset (S2-P0/P1/P2/P5, S3-P0, S4-P0/P2, S7-P2) was never run |
| liveness_comparison (S3-P2 TCP hop) | **NOT_EVALUATED** | Stage36 was retrospective and had no margin |
| integration_decision | none (REJECTED as tested in Stage35; stands) | owner |
| release_certification | none; no rc.3 | |

The advisory is a warning (unsoundness), not a vulnerability, and its
exposure was judged unreachable. That supports the normal path (9a),
not the emergency path.

## 5. Recommended decisions and rationale

**D1 (1a): vocabulary.** Machine-readable fields, one closed set each:

| Field | Values | Notes |
|---|---|---|
| `run_outcome` | `PASS`, `FAIL`, `EXCLUDED` | authoritative; produced only by the existing Stage32 and Stage34 rules |
| `failure_class` | `integrity`, `crash`, `leak`, `downgrade`, `fence`, `control_plane`, `liveness_bound`, `early_reset`, `harness` | what failed |
| `failure_signature` | `kernel_tail` (the existing rule; the run is EXCLUDED), `closewait_backpressure`, `quic_proto_0_11_18`, `unclassified` | descriptive; **never changes `run_outcome`**. `closewait_backpressure` is a FAIL with that label. |
| `provenance` | `baseline_and_candidate`, `candidate_only`, `baseline_only`, `harness` | per signature, across arms |
| `layer_a_status` | `SATISFIED`, `VIOLATED`, `INCOMPLETE` | per candidate |
| `liveness_comparison` | `LIVENESS_NONINFERIOR`, `LIVENESS_INFERIOR`, `LIVENESS_INCONCLUSIVE`, `NOT_EVALUATED` | per eligible limitation; deliberately not named PASS |
| `run_validity` | `VALID`, `INVALID` (with reason) | an invalid run keeps its raw outcome in the record but is excluded from the statistics |
| `remediation_recommendation` | `RECOMMEND_INTEGRATION`, `DO_NOT_INTEGRATE`, `DEFER` | the analyst's recommendation |
| `integration_decision` | `APPROVED`, `REJECTED`, `PENDING` | the owner's, with date |
| `release_certification` | `CERTIFIED`, `NOT_CERTIFIED`, with a per-cell `EXCLUDED` list | per release, separate stage |

A `candidate_only` signature that is `unclassified` makes Layer A
`VIOLATED`, pending diagnosis. A new failure kind is never absorbed into
a rate.

**D2 (2d):** the control is the only fair comparator. The environment,
time and host matter (Stage36 block effects). An old artifact run on
another day is a different experiment.

**D3 (3c):** whole blocks keep the harness's cell sequence identical in
both arms. Quartets balance linear drift, and the randomised
orientation prevents a systematic time-of-day pattern. Five-round
blocks match Stage36's practical size: 15 runs, about 20–30 minutes.

**D4 (4a), conditional on the owner's acceptance of 7.5 pp:** it is the
smallest margin that a practical trial (about 16 blocks, 6–8 hours)
can resolve with about 80% power at the observed baseline rate. It
still discriminates a 1.53.3-class regression.

**D5 (5a):**

- the risk difference with an upper bound answers "how much worse could
  it be?" directly;
- stratifying by seed respects the fixed, unbalanced behaviour of the
  three loss patterns (seed 2030, for example, dominates the QUIC-hop
  limitation);
- the block bootstrap covers dependence within a sandbox;
- taking the more conservative bound makes neither assumption decisive.

**D6 (6a):** a fixed size with harm-only interim looks keeps the
non-inferiority bound valid. Early acceptance would be the main
optional-stopping risk, and it is excluded.

**D7 (7a):** the legs are separated by construction, so neither can
mask the other. The Stage34 conditions stay the only route to QUIC
certification.

**D8 (8a):** pre-registration is the main defence against post hoc
margins and endpoints.

**D9 (9a, with 9b as a written emergency path):** each claim is kept to
what its evidence supports.

## 6. Statistical methodology (Recommendation)

### 6.1 Units and dependence

The unit of observation is a TCP-hop leg. There are three levels of
dependence:

- **Seeds:** the netem seeds are fixed per run position, so a seed is a
  *stratum*, not a replicate of the environment. Timing jitter makes
  outcomes vary between rounds of the same seed (Stage34 observed that
  fresh sandboxes do not even reproduce cell behaviour), but the 3 seeds
  are only 3 loss patterns.
- **Blocks:** runs within a block share daemons and sandbox state, so a
  block is a *cluster*.
- **Legs:** the QUIC leg precedes the TCP leg in each run. This
  ordering is identical in both arms, which balances its influence but
  does not remove it.

The analysis therefore never treats 120 runs as 120 independent
environments. Inference is stratified by seed and checked by
resampling whole blocks. Claims are explicitly limited to "S3-P2 as
defined, these three loss patterns, this host".

### 6.2 Primary endpoint and estimator

E1 is the indicator that a valid TCP-hop leg reached the outer bound
(`run_outcome` FAIL by the liveness bound, or EXCLUDED as a kernel
tail). The estimate is the seed-stratified risk difference
RD = p_B − p_A, with Mantel–Haenszel weights. The test:

- **H0:** RD ≥ δ.
- **H1:** RD < δ.
- **Rejection:** H0 is rejected (non-inferiority shown) when the
  one-sided 95% upper bound U < δ. U is the larger of the stratified
  Miettinen–Nurminen score bound and the block-bootstrap percentile
  bound.

### 6.3 Classification

| Condition | Result |
|---|---|
| Layer A violated | stop; `layer_a_status VIOLATED`; Layer B `NOT_EVALUATED` |
| U < δ | `LIVENESS_NONINFERIOR` |
| lower one-sided 95% bound L > 0 (the candidate is worse beyond chance) | `LIVENESS_INFERIOR`, even if U < δ (a precise small harm is reported, and the owner decides) |
| U ≥ δ and L ≤ 0 | `LIVENESS_INCONCLUSIVE` |

### 6.4 Secondary endpoints and multiplicity

The secondary endpoints are tested only if E1 shows non-inferiority, in
a fixed hierarchy: E1, then E2 (δ = 10 pp). This gatekeeping keeps the
family-wise error rate at α without splitting α. E3–E5 are descriptive:

- point estimates and 95% intervals;
- the Hodges–Lehmann shift of log latency, stratified by seed;
- no decision weight.

Per-seed results are reported, but they are never used to pick a
favourable subset.

### 6.5 Interim harm look

There is one look, after 8 blocks (4 per arm). Stop for harm if the
one-sided lower 99.9% bound of RD is above 0. This is a Haybittle–Peto
boundary, so the final analysis keeps essentially its full α. There is
no acceptance at the interim.

### 6.6 Validity and sensitivity

- **Invalid blocks:** a block with a detected suspend, a host-load
  breach (pre-registered threshold), a harness defect or a build
  mismatch is INVALID as a whole.
  - Its raw outcomes stay in the record, but it is excluded from the
    statistics.
  - It is replaced by one new block of the same arm, at the end of the
    sequence, with its paired position noted.
  - No more than 2 replacements are allowed per trial. Beyond that the
    trial is invalid.
- **Required sensitivity analyses:**
  - including the invalid blocks;
  - unstratified;
  - excluding the first block of each arm (warm-up);
  - E1 with FAIL only (without tails).
- **Concordance:** the conclusion must be stated as robust only if every
  sensitivity analysis agrees.

### 6.7 What the method does not claim

- **Not equivalence:** non-inferiority within δ is not equivalence, and
  never "no difference".
- **Inconclusive is not acceptance:** an inconclusive result is
  reported as such.
- **Limited scope:** the result says nothing about other profiles,
  cells, kernels or real networks.

## 7. Sample-size approach (Recommendation)

The prospective method:

1. Simulate the final analysis (section 6) under assumed true rates.
2. Use the A/A pilot's estimates for the baseline rate and the block
   design effect.
3. Choose the smallest whole number of quartets giving at least 80%
   power at RD = 0, with a one-sided α of 0.05.
4. Confirm by the same simulation that the false-acceptance rate at
   RD = δ is at most 5%.

The normal-approximation figures below are a planning guide only. At
these low rates the approximation is anti-conservative, so the
simulation is authoritative.

Runs per arm for non-inferiority on E1 (true RD = 0, one-sided α 0.05;
multiply by the design effect, assumed 1.25–1.5 until the pilot):

| Baseline p | δ = 5 pp | δ = 7.5 pp | δ = 10 pp | δ = 15 pp |
|---|---|---|---|---|
| 2%, power 80% / 90% | 97 / 135 | 44 / 60 | 25 / 34 | 11 / 15 |
| 4%, power 80% / 90% | 190 / 264 | 85 / 117 | 48 / 66 | 22 / 30 |
| 6%, power 80% / 90% | 279 / 387 | 124 / 172 | 70 / 97 | 31 / 43 |

At p = 4%, δ = 7.5 pp and a design effect of 1.25, the requirement is
about 106 runs per arm. Rounded up to whole blocks, that is 8 blocks
(120 runs) per arm, 16 blocks in all. At the Stage36 pace of about 1.3
minutes per run, that is about 5–6 hours of runs, 6–8 hours with setup.

What a given size can show if both arms observe the same rate. The
figure is the one-sided 95% upper bound of RD (Newcombe hybrid score,
before any design effect):

| Runs per arm | Both at 2% | Both at 4% | Both at 0 events |
|---|---|---|---|
| 30 | 10.6 pp | 10.6 pp | 8.3 pp |
| 60 | 5.6 pp | 6.6 pp | 4.3 pp |
| 120 | 3.4 pp | 4.7 pp | 2.2 pp |
| 240 | 2.4 pp | 3.2 pp | 1.1 pp |
| 480 | 1.6 pp | 2.1 pp | 0.6 pp |

So:

- **30 runs per arm** can only exclude increases of about 10 pp or
  more, which detects a 1.53.3-class regression and nothing finer.
- **60 runs per arm** can support δ = 7.5 pp only if the arms are close
  and blocks are nearly independent.
- **120 runs per arm** supports 7.5 pp with margin, and 5 pp only if
  rates are near 2%.
- **5 pp generally needs 240 or more runs per arm.**

### Stop rules (Recommendation; pre-registered)

| Trigger | Action |
|---|---|
| Any integrity mismatch, out-of-order echo, crash or panic, leak beyond the Stage32 bounds, downgrade, fence or authorization failure, in either arm | stop immediately. Candidate: Layer A `VIOLATED`. Baseline: the trial is invalid and a defect is opened (Stage32 section 7). |
| A `candidate_only` failure with an `unclassified` signature | stop and diagnose (Layer A pending) |
| A new deterministic regression: the same seed fails in the candidate in every round of 2 consecutive candidate blocks, with no such failure for that seed in the baseline | stop and diagnose |
| Interim harm boundary crossed (section 6.5) | stop: `LIVENESS_INFERIOR` |
| Invalid conditions beyond 2 replacement blocks, a build checksum mismatch, or the host or kernel changed mid-trial | stop: trial invalid, nothing concluded |
| Anything else | no stop; the trial runs to its pre-registered size |

## 8. S3-P2 TCP and QUIC handling

- **The cell is unchanged:**
  - same topology and impairments: P2, 10% loss on A's and Y's links,
    netem seeds from Stage32 section 5;
  - payload 512 KiB, bound 120 s, seed order;
  - the QUIC hop then the TCP hop, the kernel-tail rule and the F3
    rule.
- **TCP hop:**
  - the raw outcome comes from the existing rules;
  - a `CLOSE-WAIT` stall at the bound is FAIL with the
    `closewait_backpressure` signature, applied only when its evidence
    exists. Without trace or capture evidence the signature is
    `unclassified`, and the FAIL stands either way;
  - Layer B E1 counts it as a liveness failure in either arm.
- **QUIC hop:**
  - Layer A only: integrity, crash, leak and downgrade;
  - liveness is reported per seed, with seed 2030 shown separately as
    the known quinn-proto signature;
  - it never enters a Layer B statistic;
  - S3-P2 stays EXCLUDED until Stage34's conditions are met.
- **No cross-subsidy:** a candidate that improves QUIC gets no credit
  on TCP, and the reverse. A QUIC-leg failure in the candidate that is
  not of the known kind is a `candidate_only` `unclassified` failure,
  and a stop (section 7).
- **Labelling:** reports state "S3-P2 TCP-hop comparative liveness:
  <result>; S3-P2 cell: EXCLUDED (quinn-proto 0.11.18)".

## 9. Security and resource invariants

The framework operates on outcomes and never on configuration.

- No trial may change transport windows, retries, timeouts
  (`TCP_USER_TIMEOUT`, keepalive, handshake windows), connection
  budgets, admission limits, or the Pelt, Pack, Fang, Silver and
  governance semantics.
- The candidate differs from the control only by the authorised change.
  This is verified by `git diff` and by a comparison of the Linux
  dependency closures, as in Stage36.
- No allowance exists for Layer A items. A comparative result cannot
  offset a Layer A failure, and a shared Layer A failure blocks both
  arms (it is a defect, Stage32 section 7).
- Kernel limitations and dependency warnings stay visible:
  - `KNOWN_LIMITATIONS.md` keeps both S3-P2 entries;
  - an accepted advisory stays listed until it is resolved on `main`;
  - Layer B reports always restate the limitation they measure.
- The comparison-eligible list can grow only by an owner decision, with
  a merged diagnostic record of the mechanism, in the way Stage37 did
  for `CLOSE-WAIT`.

## 10. Audit and reproducibility requirements

The evidence package (under `target/`, outside Git) and the record (in
Git) must contain:

- **Source:** commits for both arms; `git diff` between them; the
  `Cargo.lock` SHA-256; the dependency closures and their diff.
- **Builds:** the build command; `werewolfd` and `werewolfctl` SHA-256
  for each arm; rustc and cargo versions; the rc.2 anchor checksum and
  whether it is equal.
- **Host:** kernel version; relevant sysctls (`net.ipv4.tcp_*`,
  `net.core.*mem*`); the CPU model and governor; the `tc` and netem
  versions; the sandbox method.
- **Network:** the exact `tc` commands and the netem seeds per run, as
  already recorded by the harness.
- **Per run:** arm, block, quartet, position, seed and wall start;
  `run_outcome`, `failure_class`, `failure_signature`, validity and
  reason; bytes, hashes and times for both legs; the boot-minus-monotonic
  clock delta; load.
- **Analysis:**
  - the per-run CSV and its SHA-256;
  - the analysis code version;
  - the pre-registration file: endpoints, δ, α, sample size, block-order
    seed, stop rules;
  - the bootstrap seed;
  - every sensitivity result;
  - every protocol deviation.
- **Exclusions:** every INVALID run and block with its reason, and every
  replacement.
- **Decisions:** every Layer A and Layer B status, the remediation
  recommendation, the owner's integration decision and its date.

The record must reproduce its tables from the CSV alone, so it is
intelligible without the diagnostic machine.

## 11. Worked example: Stage35–37

**Why Stage35 rejected tokio 1.54.1.** The handoff's rule stopped at the
first demonstrated regression, under the absolute S3-P2 gate. Run 28, a
`CLOSE-WAIT` stall at the bound, was outside the kernel-tail rule, so
it was FAIL, and 2 tails met that trial's stop rule. There was no
contemporaneous control, so "regression" could not be separated from
"baseline behaviour". At a baseline rate of 2–4%, that gate stops an
unchanged build 45–71% of the time: this is the false-rejection path.

**Why the Stage35 FAIL remains valid.** It is a correct application of
the rules in force. Run 28 did reach the bound outside the kernel-tail
definition, and its raw outcome is permanent (invariant 8). What
Stage36 and Stage37 changed is the interpretation of its provenance
(`baseline_and_candidate`, signature `closewait_backpressure`, probable,
untraced), not the outcome.

**Why Stage36 was stronger evidence.**

- It was contemporaneous, with identical source, alternating ABBA and
  BAAB blocks, both arms, and both instrumentation states.
- It showed the same failure on the baseline.
- Looked at retrospectively, its E1 counts were baseline 4 of 60
  (2 FAIL + 2 tails) against candidate 1 of 60. The one-sided 95% upper
  bound of RD is about 1.6 pp, before any design effect.
- Pooled across Stages 34–37 (150 baseline, 90 candidate runs), the
  upper bound is about 5.9 pp.

**Why Stage36 does not approve the update.**

- **No pre-registration:** the margin, endpoint and sample size were not
  fixed beforehand, so these numbers are post hoc and are shown here as
  illustration only.
- **Mixed design:** it mixed instrumented and uninstrumented campaigns
  and included a host suspend.
- **Layer A incomplete:** the Stage32 QUIC subset was never run.
- **Not a certification:** Stage36's own conclusion was "accept for
  further certification", which is not an approval.

**Why Stage37 supports a shared mechanism.**

- A traced 1.52.3 failure showed the relay correctly blocked on a
  non-writable socket, with ACK progress frozen by lost retransmissions
  under backoff, and ended by `TCP_USER_TIMEOUT`.
- Nothing in the mechanism depends on the tokio version beyond correct
  write readiness, which was observed. The shared mechanism is
  therefore proven for the traced run and probable for the others.

**How a prospective trial differs.**

- the margin, endpoints, size, order, stop rules and analysis are fixed
  first;
- all certification blocks are uninstrumented;
- suspend detection is in place;
- Layer A is complete before Layer B;
- there is a single primary endpoint;
- results are recorded in fields that cannot conflate a comparison
  with a PASS.

**False-acceptance paths and their detection:**

| Path | Detection or prevention |
|---|---|
| "The baseline fails too, so accept" | Raw outcomes stay FAIL. Layer B tests a bounded *difference*; a high shared rate is still a known limitation, not acceptance. |
| An underpowered, non-significant result read as "no regression" | Classified `LIVENESS_INCONCLUSIVE`; only U < δ counts |
| A margin or endpoint chosen after seeing data | Pre-registration; deviations reported |
| Optional stopping | Fixed size; harm-only interim |
| A stale or mismatched baseline | Contemporaneous control (D2); checksum and closure diff |
| A new failure kind hidden inside a rate | `candidate_only` `unclassified` triggers a stop and a Layer A hold |
| Widening exclusions (for example, `CLOSE-WAIT` as a tail) | Rules frozen; signatures are descriptive only |
| QUIC exclusion masking TCP, or the reverse | Legs separated (D7) |
| Instrumentation differing between arms | One instrumentation state for all certification blocks |
| Choosing the favourable endpoint or seed | One primary endpoint; hierarchy; per-seed results never used to decide |
| Suspend or load artefacts favouring one arm | Detection, block invalidation, balanced replacement |

**False-rejection paths and their detection:**

| Path | Detection or prevention |
|---|---|
| The absolute gate on a cell with a known non-zero baseline rate | Layer B for listed limitations only |
| A one-off environmental artefact | Validity rules and block replacement |
| A small chance excess read as harm | Inferiority requires L > 0 at the final analysis, or the stricter interim boundary |
| Comparing against an old campaign on another day | Contemporaneous control only |

## 12. Risks and unresolved questions

1. **Margin choice is a value judgement.** No evidence fixes δ. The
   owner's choice determines what harm is tolerated.
2. **Only three loss patterns.** The result cannot generalise beyond
   them. More seeds would change the cell, which is a Stage32 rule
   change and needs separate review.
3. **The block design effect is unknown** until the A/A pilot. If it is
   large, 16 blocks may be underpowered, and the sizing must be redone
   before the candidate trial.
4. **The QUIC leg precedes the TCP leg.** Its state (for example, setup
   resets) may influence the TCP leg in either arm. The influence is
   balanced but not removed.
5. **Host drift over 6–8 hours, and kernel updates.** A kernel change
   mid-trial invalidates the trial.
6. **Owner workload.** Each trial needs a pre-registration review and a
   decision.
7. **The emergency path (9b)** could be overused. Each use must name the
   advisory and its unmeasured risk.
8. **Unresolved: should E1 include kernel tails?** Recommended yes:
   both are deadline failures by the same mechanism family, and
   counting both is conservative. The owner may prefer separate
   endpoints, which costs power.
9. **Unresolved: should a narrower "FAIL-only" analysis be primary
   for a candidate that only touches the scheduler?** Not recommended:
   one endpoint for all trials.

## 13. Implementation milestones (conditional on approval)

None is authorised by this survey.

1. **M1:** the owner approves D1–D9, δ, α and power. The contract is
   merged as a docs-only spec.
2. **M2:** harness additions in their own PR, with tests: the
   machine-readable fields (D1), suspend detection, a block runner with
   pre-registered order, and host records. **No change to any existing
   rule, classifier, bound or payload.**
3. **M3:** an analysis tool (stratified Miettinen–Nurminen, block
   bootstrap, simulation-based sizing). Unit-tested against published
   reference values and synthetic data, in its own PR.
4. **M4:** an A/A pilot (control against control, the same design, for
   example 8–16 blocks), to estimate the design effect and confirm the
   false-difference rate. Its own record.
5. **M5:** the tokio 1.54.1 trial.
   - **Layer A first:** the full Stage35 gate list plus the Stage32 QUIC
     subset.
   - **Then Layer B:** a pre-registration, then the trial.
   - **Record:** a docs-only record.
6. **M6:** the owner's integration decision.
7. **M7:** separate release certification for any rc.3, under its own
   survey.

## 14. Decision table for owner review

| # | Decision | Options | Recommended |
|---|---|---|---|
| D1 | Terminology | 1a fields · 1b prose · 1c "comparative PASS" | **1a** (section 5 vocabulary) |
| D2 | Reference baseline | 2a current control · 2b rc.2 artifact · 2c both · 2d control + rc.2 as anchor only | **2d** |
| D3 | Design | 3a single-run alternation · 3b two campaigns · 3c counterbalanced quartets of 15-run blocks · 3d adaptive | **3c**: 8 blocks per arm, uninstrumented, suspend detection |
| D4 | Margins | 4a E1 7.5 pp + E2 10 pp · 4b E1 5 pp · 4c E1 10 pp · 4d withhold | **4a**, or **4d** if no margin is acceptable |
| D5 | Statistics | 5a stratified MN + block bootstrap, the more conservative bound · 5b Fisher p · 5c Bayesian | **5a**, one-sided 95% |
| D6 | Size and stopping | 6a fixed, simulation-sized, harm-only interim · 6b full group-sequential · 6c run until acceptable | **6a**, with the A/A pilot first |
| D7 | S3-P2 legs | 7a separated by construction · 7b pause S3-P2 | **7a** |
| D8 | Evidence | 8a pre-registration + package · 8b retrospective | **8a** |
| D9 | Advisory and release | 9a five ordered statuses · 9b emergency path | **9a**, with 9b as a written exception |
| — | Comparison-eligible list | | the S3-P2 encrypted-TCP hop only |
| — | Kernel tails in E1 | include · separate | include |
| — | A/A pilot before any candidate trial | yes · no | yes |

Implementation authorised: **NO**. Tokio promotion authorised: **NO**.
Release certification authorised: **NO**.

## Owner decisions (2026-11-02)

This section records the owner's decisions on D1–D9. The survey text
above is unchanged. Where this section differs from it, **this section
supersedes it**, and each superseded passage is named. The proposals
first drafted in PR #104 ("Proposed revisions, awaiting owner
confirmation") are replaced by the final decisions below.

| Decision | Status |
|---|---|
| D1, machine-readable outcomes | **Final** (1a, with the classification rules of D1 below) |
| D2, contemporaneous control, rc.2 as historical anchor | **Approved** (2d) |
| D3, counterbalanced 15-run blocks | **Approved conditionally** (3c): subject to pilot evidence |
| D4, margins | **Provisional**: E1 7.5 pp, E2 10 pp, one-sided α 5%, power at least 80%; not final acceptance thresholds |
| D5, statistical method | **Not approved** as a certification method; 5a and the proposed 5a′ are both withdrawn as binding tests (D5 below) |
| D6, fixed sample, harm-only interim | **Approved in principle** (6a): conditional on simulation |
| D7, TCP and QUIC legs separate | **Approved** (7a) |
| D8, preregistration and reproducible evidence | **Approved** (8a) |
| D9, distinct remediation and release decisions | **Approved** (9a), with the controlled 9b emergency exception below |

**Scope.** The only comparison-eligible limitation is the **S3-P2
encrypted-TCP hop**. Kernel tails count in E1, while their raw outcome
stays EXCLUDED.

**Consequence.** No candidate can currently receive
`LIVENESS_NONINFERIOR`, because D4 and D5 are unfinished. Until they
are finished, a candidate trial may report estimates only (option 4d in
effect). A separate statistical validation specification, to be proposed as
`docs/STAGE38_STATISTICAL_VALIDATION_SPEC.md`, will design that
remaining work. Implementation, tokio promotion and release certification are
**not authorised**.

### D1: classification rules (final)

**Raw outcomes stay authoritative.**

- Every leg keeps its raw `PASS`, `FAIL` or `EXCLUDED` from the
  existing Stage32 and Stage34 rules. No comparative result changes a
  raw outcome. A `CLOSE-WAIT` timeout stays FAIL.
- **E1 events:** a valid TCP-hop leg is an E1 event when it reaches the
  deadline and is either:
  - `FAIL` for liveness (`failure_class` `liveness_bound`), or
  - `EXCLUDED` under the existing kernel-tail rule
    (`failure_signature` `kernel_tail`).
- **Invalid runs:** a harness-invalid run enters neither E1's numerator
  nor its denominator.
  - Every invalidation needs an independently established validity
    reason (D3 and D8): a detected suspend, a build mismatch or a
    demonstrable harness defect.
  - A run is never excluded because it failed.

**Layer A vocabulary.** The values are `SATISFIED`, `VIOLATED`, `HELD`
and `INCOMPLETE`.

- **What `HELD` means:** a suspected security or correctness violation,
  or a genuinely new, unexplained failure class, needs investigation.
  It does not mean a violation is proven.
- **What does not trigger `HELD`:**
  - an ordinary S3-P2 liveness timeout without detailed tracing;
  - a known failure whose precise kernel mechanism was not captured. It
    stays in its established class.
- **What does trigger it:** a genuinely new candidate-only failure kind.
  It stops the trial for investigation.
- **After a stop:** a stopped trial is never resumed. If the issue is
  resolved, a new trial needs a new preregistration.
- **Supersedes:** survey section 5, "A `candidate_only` failure with an
  `unclassified` signature makes Layer A `VIOLATED`"; the stop-rule
  table row "A `candidate_only` failure with an `unclassified`
  signature"; and the first PR #104 proposal (`HELD` on any
  unclassified candidate failure).

**Failure signatures are descriptive, never acceptance criteria.**

- `closewait_backpressure` is used only where trace, or packet and
  socket evidence, supports that mechanism.
- Otherwise the leg keeps its known broad failure class (for example,
  `failure_class` `liveness_bound`), with `failure_signature`
  `unclassified` for the unsupported mechanism. An unclassified
  *cause* is not an unclassified *failure class*.
- `candidate_only` means observed only in the candidate sample. It does
  not establish causality.

**Comparative results: two independent findings, both always
reported.**

Non-inferiority status, with U and L the prespecified one-sided
confidence bounds of RD = p_B − p_A:

| Condition | Result |
|---|---|
| U < δ | `LIVENESS_NONINFERIOR` |
| L > δ | `LIVENESS_INFERIOR` |
| neither | `LIVENESS_INCONCLUSIVE` |

Observed deterioration: `WORSENING_DETECTED` whenever the prespecified
lower bound L > 0. So a candidate can be non-inferior within δ and
still show a smaller, statistically detectable deterioration, and both
findings are visible.

When Layer A is `HELD`, `VIOLATED` or `INCOMPLETE`, final comparative
certification is blocked. Statistical estimates may be kept as
diagnostic evidence but are never presented as an approved Layer B
result.

These two findings supersede survey section 6.3 and the first PR #104
proposal for an ordered single result. In particular, `LIVENESS_INFERIOR`
now means L > δ, not L > 0.

### D4: provisional margins

| Endpoint | Provisional margin |
|---|---:|
| E1: deadline liveness events, kernel tails included | 7.5 pp |
| E2: clean early resets | 10 pp |

- **Design intent:** one-sided α 5% and power at least 80%.
- **Scope:** the eligible S3-P2 encrypted-TCP hop only.
- **Not final:** these are not approval thresholds. Statistical
  validation must show whether they can be evaluated reliably with a
  practical experiment.
- **Fallback:** if no defensible method or sample size is found,
  comparative certification is withheld.

### D5: direction approved, estimator not approved

5a (the larger of two upper bounds) and the proposed 5a′ are **not
approved** as binding tests. The owner's reasons:

- an intraclass correlation estimate is not itself a design effect;
- an upper 80% correlation estimate does not by itself give adequate
  confidence-bound coverage;
- a fallback design effect of 1.5 is not inherently conservative;
- eight blocks per arm may be too few for a reliable cluster bootstrap;
- an exact interval derived under independence does not account for
  clustering.

The research candidates are:

- seed-stratified risk-difference score intervals with a defined
  cluster adjustment;
- cluster-aware or small-sample-corrected procedures;
- methods respecting the randomised block and quartet design;
- conservative procedures for sparse or zero-event samples.

**Process:**

- A single primary method is selected only after offline simulation
  shows acceptable operating characteristics.
- Sensitivity analyses never override the prespecified primary method.
- **Supersedes:** survey sections 4 (D5, 5a), 6.2 (the "larger of the
  two" rule), 6.5 (the interim boundary, now unvalidated) and 7 (the
  sizing table, a planning illustration only), and the 5a′ proposal.

### D9b: emergency security exception (final)

Layer B may be bypassed as a prerequisite only when all of the following
hold:

1. there is a confirmed, actionable security vulnerability;
2. exposure is reachable or cannot reasonably be ruled out;
3. Layer A is fully `SATISFIED`;
4. the owner authorises the exception explicitly in writing;
5. the decision names the advisory and its security urgency;
6. the unmeasured comparative liveness risk is documented;
7. the exception has a review and expiry deadline;
8. a follow-up preregistered comparative trial is required.

**Limits of the exception:**

- It waives no Layer A requirement.
- It does not certify a release.
- **Expiry without follow-up:** if the deadline passes without adequate
  follow-up, further release approvals relying on the exception are
  blocked pending owner review.

RUSTSEC-2026-0386 does not qualify under the current assessment: it is
an unsoundness warning judged unreachable.

### Correction: historical probability explanation

The survey's sections 1, 2 and 11 conflated two endpoints when they said
that Stage35's gate "would have stopped the unchanged baseline itself in
about 45–71%" of 30-run trials. The two endpoints are separated here.
The original sentences stay as written above; this correction
supersedes them.

| Endpoint | Historical baseline (tokio 1.52.3, Stages 34–37) | P(at least 1 in 30), independent-run model |
|---|---|---|
| Raw deadline FAILs (the existing no-FAIL rule) | 3 in 150, about 2% | 1 − 0.98³⁰ ≈ 45% |
| Combined E1 events (FAIL plus kernel tail; the proposed comparative endpoint) | 6 in 150, about 4% | 1 − 0.96³⁰ ≈ 71% |

These figures need four caveats:

1. **The rule counts only one endpoint.** The existing S3-P2 no-FAIL
   rule counts raw FAILs only. The 45% figure is the relevant
   illustration for it; the 71% figure concerns E1, which is a
   comparative endpoint, not the rule.
2. **Stage35 had a second stop.** It had a separately specified
   kernel-tail stop threshold (more than one tail in 30), whose chance
   under an independent-run model at a 2% tail rate is
   1 − 0.98³⁰ − 30 × 0.02 × 0.98²⁹ ≈ 12%. Neither figure alone gives
   the chance that the whole Stage35 gate stops.
3. **These are illustrations, not validated probabilities.** Runs
   within a block are dependent, and the pooled baseline mixes
   campaigns, so these binomial figures are not the actual campaigns'
   probabilities.
4. **The verdict stands.** The historical Stage35 verdict (REJECTED) is
   unchanged.
