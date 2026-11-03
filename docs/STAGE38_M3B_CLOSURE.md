# Stage38 M3b — closure and certification policy record

**Status: Stage38 M3b CLOSED.** Comparative certification is
**SUSPENDED** (`COMPARATIVE_CERTIFICATION_SUSPENDED`). No practical
validated primary method exists, and M4 is not authorised.

Recorded: `M3B-B — NO_PRACTICAL_VALIDATED_METHOD`. This accepts a
research conclusion. It is **not** approval of C6 or of any other
statistical certification procedure.

Documentation only. No production source, harness, dependency or
statistical-engine change. The M3 and M3b reports, results, numerical
tables and simulation evidence are unchanged, apart from one
cross-reference added to the M3b report.

## 1. Baseline

- `main` `58a8c8d390d747915d5815cd797ad5edaf918575`.
- tokio 1.52.3; S3-P2 EXCLUDED (`KNOWN_LIMITATIONS.md`); tag
  `v1.0.0-rc.2` unchanged (object `ec10180`, commit `b192158`).

## 2. Integration reference

PR #107 (Stage38 M3b) is merged; its merge commit is `58a8c8d`. It
contains the frozen validation plan (`920716d`), the tooling, the
results and `docs/STAGE38_M3B_RARE_EVENT_RESEARCH.md`. Its history is
not rewritten.

## 3. Final research classification

The owner accepts the M3b investigation as completed, with these
findings:

- C0–C5 and C7 failed the M3 statistical criteria in the tested
  scenarios.
- R2 and R3 failed false-acceptance control.
- R1 and R1EB were conservative but did not improve practical power.
- C6 remained the strongest operational candidate among the evaluated
  methods.
- C6 requires multi-day experiments to reach the proposed power.
- No method showed both satisfactory statistical properties and
  operational practicality under the adopted assumptions.
- Host-dependent invalidation creates a potential selection-bias
  problem.
- The E2 hierarchy depends on a future policy decision for zero-event
  samples.

## 4. Scientific clarifications

Where the M3b report calls C6 or R1 "valid" without qualification, the
qualifications below govern. The report's numbers stand as recorded.

| | Mathematical guarantee | Simulation-based validation | Unverified real-world assumption | Operational suitability |
|---|---|---|---|---|
| C6 | none finite-sample; a variance bound (block proportion in [0, 1]) plus a normal approximation, for independent blocks | false acceptance and coverage acceptable across the simulated G1, G2 and G3 scenarios | that real blocks resemble the simulated families (independence, or the simulated dependence) | not suitable: about 32–64 quartets (41–90 h) |
| R1 | finite-sample, **only** under its assumptions: independent blocks whose conditional mean within an arm equals the arm mean | conservative in every simulated scenario, G3 included | that the conditional-mean structure holds on a real host; drift or host dependence breaks the guarantee | not suitable: less powerful than C6 at every size |
| R2, R3 | none (asymptotic only) | **rejected**: false acceptance 9–24% at δ, under-coverage in hundreds of cells, in every family | — | — |

### 4.1 Scope of statistical validity

- **C6** showed acceptable false-acceptance control and confidence-bound
  coverage across the simulated G1, G2 and G3 scenarios. That is
  empirical evidence within the evaluated model families. It is **not**
  a distribution-free, finite-sample guarantee for arbitrary dependent
  or nonstationary blocks.
- **R1's** mathematical guarantees hold only under its stated
  assumptions, including the required conditional-mean structure of
  block observations. Its satisfactory simulated performance under G3
  does not establish validity for every possible host-dependence
  process.
- **R2 and R3** keep their rejection and failure cases unchanged
  (M3b sections 8 and 13). Nothing here softens their reported
  under-coverage or false-acceptance rates.

### 4.2 The lower bound on required blocks (M3b section 5.1)

The result is preserved: (1 − δ)^n_B ≤ α. At δ = 0.075 and α = 0.05, the
all-or-nothing null construction requires **at least 39 candidate
blocks** for a level-5% test that rejects on all-zero data.

The construction's assumptions:

- candidate blocks are independent;
- within-block dependence may be arbitrary;
- block proportions lie in [0, 1];
- the null model includes all-or-nothing candidate blocks;
- the control block proportions are zero;
- the test declares non-inferiority when both arms contain no E1 event.

This is a lower bound **within that unrestricted block-dependence
model**, not a universal impossibility theorem for every statistical
design.

- **Methods with more assumptions:** methods that assume something about
  failure clustering may have different requirements. Any such
  assumption would need independent justification before it could be
  used for Werewolf certification.
- **Data cannot supply it:** neither the existing rare-event data nor an
  A/A pilot can reliably establish such an assumption (spec section 7;
  M3b section 5.1).

### 4.3 Host-dependent invalidation

The findings are kept (M3b section 12), with their uncertainty stated.
In the 5% host-driven invalidation scenario at 48 quartets:

- **Six replacement quartets:** about 2.3% false acceptance among 217
  valid experiments, with a Monte Carlo upper bound of about 4.8%.
- **Two replacement quartets:** the reported "about 9%" is one false
  acceptance among only 11 valid experiments. That estimate is extremely
  uncertain. It does **not** establish a true false-acceptance rate
  above 5%, and it does not show acceptable error control either.

**The supported architectural concern:** host-dependent invalidation may
select against difficult network conditions, and bias the
candidate-control comparison in the candidate's favour. The bias appears
in every policy simulated: −0.9 to −4.9 pp in the RD estimate.

**On caps:** a larger replacement cap improves completion under
independent invalidation. It does **not** establish unbiased inference
under informative invalidation. No replacement-policy change is
approved.

## 5. Owner decisions

| Item | Decision |
|---|---|
| M3b research | Accepted and closed |
| Comparative certification | Suspended |
| D4 margins | Provisional; not final |
| D5 primary method | Not approved |
| C6 multi-day campaign | Not authorised |
| M4 A/A pilot | Not authorised |
| V5 replacement cap | Remains 2 |
| Proposed cap of 6 | Not approved |
| E1 margin increase to 10 pp | Not approved |
| E2 zero-event endpoint | Inconclusive; no automatic acceptance |
| New M3c estimator research | Not authorised |
| Tokio 1.54.1 promotion | Not authorised |
| rc.3 certification | Not authorised |

These are unchanged:

- the Layer A absolute security and correctness requirements;
- the Stage32 and Stage34 certification rules (kernel tail, F3, the
  120 s bound, the 512 KiB payload);
- historical PASS, FAIL and EXCLUDED outcomes, which remain
  authoritative.

## 6. E2 zero-event handling

The policy:

- **Inconclusive:** if E2 has no observed event in either arm, the
  current procedure gives an inconclusive E2 comparison. The absence of
  events alone does not prove non-inferiority.
- **No compensation either way:** an E1 result cannot compensate for an
  inconclusive E2 result, and an improved E2 result cannot compensate for
  an E1 failure.
- **Future change:** a validated confidence procedure might support a
  different conclusion for zero-event samples, but it would need separate
  architectural approval. None is authorised here.

## 7. Comparative certification status

**`COMPARATIVE_CERTIFICATION_SUSPENDED`.** No result may be reported as
`LIVENESS_NONINFERIOR`, under any method, by any activity.

**Normal engineering continues.** Closing M3b does not block dependency
maintenance:
- security vulnerability investigation;
- dependency impact analysis;
- functional regression testing;
- QUIC regression diagnosis;
- transport correctness fixes;
- the existing security and resource gates;
- documentation and architectural review.

None of these may use the unapproved Layer B method to claim
`LIVENESS_NONINFERIOR`.

**tokio and the emergency exception:**
- tokio 1.54.1 remains unpromoted.
- The documented treatment of RUSTSEC-2026-0386 remains in force.
- The narrowly controlled D9b emergency exception remains available,
  only under its approved conditions. This closure does not invoke it.

## 8. Remaining unresolved assumptions

- **Real blocks:** whether real S3-P2 blocks are independent, or
  resemble any simulated dependence family.
- **Invalidation:** whether real invalidation (suspend, harness failure,
  host load) is independent of host network conditions. If it is
  informative, the comparison may be biased toward the candidate.
- **C6:** its behaviour outside G1–G3. It has no finite-sample proof,
  even for independent blocks.
- **R1:** whether its conditional-mean assumption holds on a real host.
- **E2:** power at practical sizes (0.76–0.81 at 48 quartets), and the
  zero-event case.

## 9. Conditions for any future reconsideration

Reopening Stage38 needs new evidence or a materially different
experimental design. A proposal must at least address:

1. a defensible statistical method;
2. adequate error control and coverage;
3. practically achievable power;
4. dependence between blocks;
5. host-driven invalidation bias;
6. E2 endpoint handling, including zero events;
7. experiment completion probability;
8. explicit owner approval of D4 and D5.

Changing a margin merely to shorten an experiment is not sufficient
justification. Neither is changing a replacement cap merely to improve
completion. The default remains `COMPARATIVE_CERTIFICATION_SUSPENDED`.

## 10. Authorisation boundaries

**Not authorised:**
- D4 finalisation or D5 approval;
- any C6 or other Layer B campaign;
- the M4 A/A pilot;
- M3c research;
- V5 or margin changes;
- harness changes;
- tokio promotion;
- release certification or rc.3.

**Authorised:** only this record, its cross-reference and the STATUS
entry.
