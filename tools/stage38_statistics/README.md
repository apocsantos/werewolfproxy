# Stage38 M3: offline statistical simulation

Research tooling for the Stage38 Layer B comparative method. It is not part
of Werewolf, the chaos harness or any certification gate. Nothing here is
imported by them, and it adds nothing to the Rust workspace. The record is
`docs/STAGE38_M3_SIMULATION_REPORT.md`.

## Setup

Python 3.13 with the pinned packages in `requirements.txt`, in a virtual
environment outside the repository:

```sh
python3 -m venv target/stage38-m3/venv
target/stage38-m3/venv/bin/pip install -r tools/stage38_statistics/requirements.txt
```

statsmodels is used only as a reference in the tests.

## Tests

```sh
cd tools/stage38_statistics
../../target/stage38-m3/venv/bin/python -B -m unittest discover -s tests -v
```

The tests cover:
- the design (balance, block and seed structure, replacement);
- marginal calibration (quadrature and Monte Carlo);
- E1/E2 exclusivity and reproducible streams;
- reference values: the restricted MLE against a numerical optimiser, the
  MN score against statsmodels at 0, Welch and paired t against SciPy, CR2
  against Welch, the beta-binomial table against SciPy;
- sparse events (zero, one-sided zero, single, all);
- symmetry, sign reversal and monotonicity;
- the Layer B findings.

## Runs

```sh
cd tools/stage38_statistics
export OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1
PY=../../target/stage38-m3/venv/bin/python
$PY -B -m s38.runner config/screening.json    ../../target/stage38-m3/screening    --workers 4
$PY -B -m s38.runner config/confirmation.json ../../target/stage38-m3/confirmation --workers 4
$PY -B -m s38.runner config/power.json        ../../target/stage38-m3/power        --workers 4
$PY -B -m s38.runner config/interim.json      ../../target/stage38-m3/interim      --workers 4
$PY -B -m s38.analysis ../../target/stage38-m3/screening ../../target/stage38-m3/screening
```

- **Resuming:** each scenario writes one JSON file. A rerun skips files whose
  configuration hash matches, so an interrupted run resumes.
- **Random streams:** `SeedSequence(root_seed, spawn_key=(scenario, chunk))`,
  so results do not depend on worker count. Screening and confirmation use
  different root seeds.
- **Failures:** a replicate is never regenerated. Numerical failures and
  degenerate cases are counted per method.

## Layout

| Module | Role |
|---|---|
| `s38/design.py` | blocks, seed order, balanced ABBA/BAAB quartets, whole-quartet replacement (cap 2) |
| `s38/generator.py` | block-level logistic model, marginal calibration, E1/E2 outcomes |
| `s38/estimators.py` | C0–C7 and the common degenerate-case rule |
| `s38/scenarios.py` | the screening, confirmation and power grids |
| `s38/runner.py` | the resumable, deterministic runner |
| `s38/analysis.py` | operating characteristics with Clopper–Pearson Monte Carlo bounds |
| `s38/classify.py` | the Layer B findings (final D1 rules) |
