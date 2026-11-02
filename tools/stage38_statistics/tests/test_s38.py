"""Unit and reference tests for the Stage38 M3 simulator (handoff section 10).

    cd tools/stage38_statistics && python -m unittest discover -s tests -v
"""
import json
import os
import tempfile
import unittest

import numpy as np
from scipy import stats
from scipy.optimize import minimize_scalar

from s38 import classify, design, estimators as est, generator as gen, runner

ALL = list(est.METHODS)
TH = [-0.05, 0.0, 0.05, 0.075, 0.10]


def _load(path):
    with open(path) as f:
        return json.load(f)


def manual(a_counts, b_counts, quartets=4):
    """Data with given per-block E1 counts (lists of 2*quartets ints each)."""
    reps = 1
    outcome = np.zeros((reps, quartets, 4, 15), np.int8)
    orient = np.array([[design.ABBA, design.BAAB] * (quartets // 2)], np.int8)
    arm = design.arms_of(orient)
    ia = ib = 0
    for q in range(quartets):
        for pos in range(4):
            k = a_counts[ia] if arm[0, q, pos] == 0 else b_counts[ib]
            if arm[0, q, pos] == 0:
                ia += 1
            else:
                ib += 1
            outcome[0, q, pos, :k] = 1
    return est.Data({'outcome': outcome, 'arm': arm, 'orient': orient})


class Design(unittest.TestCase):
    def test_balanced_and_reproducible(self):
        o1 = design.balanced_orientations(np.random.default_rng(7), 500, 8)
        o2 = design.balanced_orientations(np.random.default_rng(7), 500, 8)
        np.testing.assert_array_equal(o1, o2)
        self.assertTrue(((o1 == design.ABBA).sum(1) == 4).all())

    def test_block_and_seed_structure(self):
        sim = gen.simulate(np.random.default_rng(1), gen.Scenario(0.04, 0.0, 0.5, 4), 50)
        self.assertEqual(sim['outcome'].shape, (50, 4, 4, 15))
        self.assertTrue(((sim['arm'] == 0).sum((1, 2)) == 8).all())     # 8 blocks per arm
        self.assertEqual(list(design.SEED_POS[:6]), [0, 1, 2, 0, 1, 2])
        self.assertTrue((np.bincount(design.SEED_POS) == 5).all())        # equal seed weights

    def test_replacement(self):
        rng = np.random.default_rng(3)
        orient, analysed, valid, used = design.schedule(rng, 4000, 4, 0.10)
        self.assertTrue((used <= design.MAX_REPLACEMENTS).all())
        for r in np.flatnonzero(valid)[:500]:
            self.assertEqual(len(set(analysed[r])), 4)
            o = orient[r, analysed[r]]
            self.assertEqual(int((o == design.ABBA).sum()), 2)          # balance kept
        self.assertTrue((~valid).any())                                   # cap exceeded happens
        o0, a0, v0, u0 = design.schedule(np.random.default_rng(3), 100, 4, 0.0)
        self.assertTrue(v0.all() and (u0 == 0).all())
        np.testing.assert_array_equal(a0, np.tile(np.arange(4), (100, 1)))

    def test_patterns(self):
        self.assertEqual(design.balanced_patterns(4).shape, (6, 4))
        self.assertAlmostEqual(est.c5_attainable(4), 1 / 6)
        self.assertAlmostEqual(est.c5_attainable(8), 1 / 70)


class Generator(unittest.TestCase):
    def test_marginal_calibration(self):
        for kw in ({'sigma': 0.0}, {'sigma': 1.5, 'asym': 0.5},
                   {'sigma': 1.0, 'phi': 0.5, 'seeds': 'seed2030x3'},
                   {'sigma': 1.5, 'drift': 'linear', 'tau': 0.5},
                   {'sigma': 0.5, 'drift': 'step', 'tau': 0.5}):
            sc = gen.Scenario(0.04, 0.05, quartets=4, **kw)
            for arm in (0, 1):
                sd = np.hypot(sc.sigma if arm == 0 else sc.sigma_b, sc.sd_w)
                t = np.arange(16) / 15
                for s in range(3):
                    m = gen.marginal(sc.mu[arm][s], sd, sc.tau * gen.drift_shape(sc.drift, t))
                    self.assertAlmostEqual(m, sc.rates[arm][s], places=9)
            sim = gen.simulate(np.random.default_rng(11), sc, 20000)
            d = est.Data(sim)
            for x, target in ((d.a, sc.rates[0].mean()), (d.b, sc.rates[1].mean())):
                se = np.sqrt(target * (1 - target) / x.size) * 8                     # generous for clustering
                self.assertLess(abs(x.mean() - target), max(se, 0.002), kw)

    def test_outcome_codes_exclusive(self):
        sim = gen.simulate(np.random.default_rng(5), gen.Scenario(0.06, 0.0, 0.0, 4), 2000)
        o = sim['outcome']
        self.assertTrue(set(np.unique(o)) <= {0, 1, 2})
        e2_given_not_e1 = (o == 2).sum() / (o != 1).sum()
        self.assertAlmostEqual(e2_given_not_e1, gen.Q_E2, delta=0.005)

    def test_true_rd_and_infeasible(self):
        self.assertAlmostEqual(gen.Scenario(0.04, 0.075, 1.0, 4).true_rd, 0.075)
        with self.assertRaises(ValueError):
            gen.Scenario(0.02, -0.02, 0.0, 4, seeds='seed2030x3')

    def test_rng_streams_reproducible(self):
        cfg = {'reps': 600, 'chunk': 200, 'root_seed': 99, 'methods': ['C0', 'C2']}
        cell = dict(p_a=0.04, rd=0.0, sigma=1.0, quartets=4)
        with tempfile.TemporaryDirectory() as d1, tempfile.TemporaryDirectory() as d2:
            r1 = _load(runner.run_scenario((3, 'T', cell, cfg, d1)))
            r2 = _load(runner.run_scenario((3, 'T', cell, cfg, d2)))
            self.assertEqual(r1['methods']['C0']['rej_u'], r2['methods']['C0']['rej_u'])
            cfg2 = dict(cfg, root_seed=100)
            r3 = _load(runner.run_scenario((3, 'T', cell, cfg2, d1)))
            self.assertNotEqual(r1['hash'], r3['hash'])


class References(unittest.TestCase):
    def test_restricted_mle_is_the_constrained_maximum(self):
        for x1, n1, x2, n2, dl in ((7, 60, 3, 60, 0.05), (0, 40, 0, 40, 0.075), (12, 90, 2, 90, -0.05),
                                   (3, 75, 9, 75, 0.10)):
            p1, p2 = est.fm_restricted(np.float64(x1), n1, np.float64(x2), n2, dl)
            lo, hi = max(0, -dl) + 1e-12, min(1, 1 - dl) - 1e-12
            ll = lambda q: -(stats.binom.logpmf(x1, n1, q + dl) + stats.binom.logpmf(x2, n2, q))
            r = minimize_scalar(ll, bounds=(lo, hi), method='bounded', options={'xatol': 1e-12})
            self.assertLessEqual(ll(float(p2)), r.fun + 1e-7)

    def test_mn_matches_statsmodels_at_zero(self):
        from statsmodels.stats.proportion import test_proportions_2indep
        for x1, x2 in ((7, 3), (0, 4), (15, 15)):
            z = est.mn_z(np.array([[x1]]), 60, np.array([[x2]]), 60, 0.0)[0]
            ref = test_proportions_2indep(x1, 60, x2, 60, value=0, method='score', compare='diff',
                                          correction=True).statistic
            self.assertAlmostEqual(z, ref, places=9)

    def test_welch_matches_scipy(self):
        d = manual([0, 1, 0, 2, 0, 0, 1, 0], [1, 3, 0, 2, 4, 1, 0, 2])
        ru, rl, st = est.c2_block_summary(d, [0.0], alpha=0.05)
        res = stats.ttest_ind(d.block_b[0], d.block_a[0], equal_var=False)
        lo = res.confidence_interval(confidence_level=0.90).low     # one-sided 95% lower bound
        self.assertEqual(bool(rl[0, 0]), lo > 0)
        _, rl2, _ = est.c2_block_summary(d, [lo - 1e-9, lo + 1e-9])
        self.assertEqual(list(rl2[0]), [True, False])

    def test_quartet_t_matches_scipy(self):
        d = manual([0, 1, 0, 2, 0, 0, 1, 0], [1, 3, 0, 2, 4, 1, 0, 2])
        ci = stats.ttest_1samp(d.quartet_d[0], 0).confidence_interval(confidence_level=0.90)
        _, rl, _ = est.c3_quartet_paired(d, [ci.low - 1e-9, ci.low + 1e-9])
        self.assertEqual(list(rl[0]), [True, False])

    def test_cr2_reduces_to_welch_without_seed_terms(self):
        d = est.Data(gen.simulate(np.random.default_rng(2), gen.Scenario(0.06, 0.0, 1.0, 4), 20))
        Mx, ga, gb, df = est.cr2_design(d.nb, False)
        y = np.concatenate([d.a, d.b], 1).reshape(20, -1).astype(float)
        beta = y @ Mx.T
        ea = d.a - beta[:, 0][:, None, None]
        eb = d.b - (beta[:, 0] + beta[:, 1])[:, None, None]
        v4 = ((ea @ ga) ** 2).sum(1) + ((eb @ gb) ** 2).sum(1)
        v2 = d.block_a.var(1, ddof=1) / d.nb + d.block_b.var(1, ddof=1) / d.nb
        np.testing.assert_allclose(v4, v2, rtol=1e-10, atol=1e-15)
        self.assertAlmostEqual(df, 2 * d.nb - 2, places=8)

    def test_cr2_point_estimate_is_mean_difference(self):
        d = est.Data(gen.simulate(np.random.default_rng(4), gen.Scenario(0.04, 0.05, 0.5, 4), 50))
        Mx, *_ = est.cr2_design(d.nb, True)
        y = np.concatenate([d.a, d.b], 1).reshape(50, -1).astype(float)
        np.testing.assert_allclose((y @ Mx.T)[:, 1], d.b.mean((1, 2)) - d.a.mean((1, 2)), atol=1e-12)

    def test_beta_binomial_table_matches_scipy(self):
        t = est._bb_table()
        i, j = 10, 12
        mu, rho = est.C7_MU[i], est.C7_RHO[j]
        a, b = mu * (1 - rho) / rho, (1 - mu) * (1 - rho) / rho
        np.testing.assert_allclose(t[i, j], stats.betabinom.logpmf(np.arange(16), 15, a, b), rtol=1e-9)
        np.testing.assert_allclose(t[i, 0], stats.binom.logpmf(np.arange(16), 15, mu), rtol=1e-9)


class Behaviour(unittest.TestCase):
    def assert_defined(self, d):
        for m in ALL:
            ru, rl, st = est.METHODS[m](d, TH)
            self.assertEqual(ru.dtype, bool)
            self.assertTrue(set(np.unique(st)) <= {est.OK, est.DEGENERATE, est.FAILED, est.NA}, m)
            self.assertFalse((ru & rl).any(), m)                            # never both at one d0
            bad = st != est.OK
            self.assertFalse(ru[bad].any() or rl[bad].any(), m)              # degenerate: no rejection

    def test_zero_events(self):
        d = manual([0] * 8, [0] * 8)
        self.assert_defined(d)
        for m in ALL:                                                    # the common rule
            ru, rl, st = est.METHODS[m](d, TH)
            self.assertEqual(st[0], est.DEGENERATE, m)
            self.assertFalse(ru.any() or rl.any(), m)

    def test_one_sided_zero_and_single_event(self):
        self.assert_defined(manual([0] * 8, [1, 0, 0, 0, 0, 0, 0, 0]))
        self.assert_defined(manual([0] * 8, [2, 3, 0, 1, 0, 0, 0, 4]))
        self.assert_defined(manual([1, 0, 0, 0, 0, 0, 0, 1], [0] * 8))

    def test_all_events(self):
        d = manual([15] * 8, [15] * 8)
        self.assert_defined(d)
        for m in ALL:
            self.assertEqual(est.METHODS[m](d, TH)[2][0], est.DEGENERATE, m)

    def test_identical_arms_give_zero_estimate(self):
        d = manual([1, 0, 2, 0, 1, 3, 0, 0], [1, 0, 2, 0, 1, 3, 0, 0])
        for m in ALL:
            ru, rl, _ = est.METHODS[m](d, [0.0])
            self.assertFalse(ru[0, 0] or rl[0, 0], m)

    def test_sign_reversal_on_swap(self):
        a, b = [0, 1, 0, 2, 0, 0, 1, 0], [1, 3, 0, 2, 4, 1, 0, 2]
        d, s = manual(a, b), manual(b, a)
        th = [-0.10, -0.075, -0.05, 0.0, 0.05, 0.075, 0.10]
        neg = [-x for x in th]
        for m in ('C0', 'C1', 'C2', 'C3', 'C4', 'C6', 'C7'):
            ru, rl, _ = est.METHODS[m](d, th)
            ru_s, rl_s, _ = est.METHODS[m](s, neg)
            np.testing.assert_array_equal(ru[0], rl_s[0], m)
            np.testing.assert_array_equal(rl[0], ru_s[0], m)

    def test_bound_monotonicity(self):
        sim = gen.simulate(np.random.default_rng(8), gen.Scenario(0.06, 0.05, 1.0, 4), 300)
        d = est.Data(sim)
        th = np.round(np.arange(-0.1, 0.2001, 0.0125), 4).tolist()
        for m in ('C0', 'C1', 'C2', 'C3', 'C4', 'C6', 'C7'):
            ru, rl, _ = est.METHODS[m](d, th)
            self.assertTrue((np.diff(ru.astype(int), axis=1) >= 0).all(), m)   # U < d0 persists upward
            self.assertTrue((np.diff(rl.astype(int), axis=1) <= 0).all(), m)

    def test_c5_cannot_reach_five_percent_with_four_quartets(self):
        sim = gen.simulate(np.random.default_rng(9), gen.Scenario(0.04, -0.02, 0.0, 4), 2000)
        ru, rl, _ = est.c5_randomisation(est.Data(sim), [0.075, 0.0])
        self.assertFalse(ru.any() or rl.any())


class Classification(unittest.TestCase):
    def test_findings(self):
        f = classify.findings
        self.assertEqual(f(True, False, False)['layer_b'], 'LIVENESS_NONINFERIOR')
        self.assertEqual(f(True, False, True)['worsening'], 'WORSENING_DETECTED')   # both visible
        self.assertEqual(f(False, True, True)['layer_b'], 'LIVENESS_INFERIOR')
        self.assertEqual(f(False, False, False)['layer_b'], 'LIVENESS_INCONCLUSIVE')
        self.assertEqual(f(True, False, False, status_ok=False)['layer_b'], 'LIVENESS_INCONCLUSIVE')
        for a in ('HELD', 'VIOLATED', 'INCOMPLETE'):
            self.assertEqual(f(True, False, False, layer_a=a)['layer_b'], 'BLOCKED')
        with self.assertRaises(ValueError):
            f(True, True, True)


if __name__ == '__main__':
    unittest.main()
