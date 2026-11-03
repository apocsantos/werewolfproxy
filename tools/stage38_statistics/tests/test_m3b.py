"""Unit and reference tests for the Stage38 M3b additions."""
import unittest
from math import comb

import numpy as np
from scipy import stats

from s38 import design, estimators as est, m3b

FAMILIES = (('G1', {}), ('G2', {'mix_pi': 0.15}), ('G3ar', {'env_rho': 0.9}), ('G3step', {}))


def wsr_reference(x, alpha, c=0.75, grid=m3b.R1_GRID):
    """Scalar loop implementation of the same betting bounds."""
    n = len(x)
    up = lo = None
    mu, dev, var_prev, lams = 0.5, 0.0, 0.25, []
    for i, xi in enumerate(x, 1):
        lams.append(np.sqrt(2 * np.log(1 / alpha) / (n * var_prev)))
        mu_i = (0.5 + sum(x[:i])) / (i + 1)
        dev += (xi - mu_i) ** 2
        var_prev = (0.25 + dev) / (i + 1)
    for m in grid:
        k_up = np.prod([1 + min(l, c / max(1 - m, 1e-12)) * (m - xi) for l, xi in zip(lams, x)])
        if up is None and k_up >= 1 / alpha:
            up = m
    for m in grid[::-1]:
        k_lo = np.prod([1 + min(l, c / max(m, 1e-12)) * (xi - m) for l, xi in zip(lams, x)])
        if lo is None and k_lo >= 1 / alpha:
            lo = m
    return (0.0 if lo is None else lo), (1.0 if up is None else up)


class Generators(unittest.TestCase):
    def test_quadrature_calibration(self):
        for fam, kw in FAMILIES:
            for extra in ({}, {'phi': 0.5, 'asym': 0.5}, {'drift': 'step', 'tau': 0.5}):
                sc = m3b.Scenario(family=fam, p_a=0.04, rd=0.05, rd2=0.05, lam2=1.0, sigma=1.0,
                                  quartets=4, **kw, **extra)
                for arm in (0, 1):
                    for s in range(3):
                        e1, e2 = sc.marginal(arm, s)
                        self.assertAlmostEqual(e1, sc.rates[arm][s], places=8, msg=(fam, extra))
                        self.assertAlmostEqual(e2, sc.e2[arm], places=8, msg=(fam, extra))

    def test_monte_carlo_marginals(self):
        for fam, kw in FAMILIES:
            sc = m3b.Scenario(family=fam, p_a=0.04, rd=0.075, rd2=0.05, lam2=1.0, sigma=1.5,
                              quartets=8, **kw)
            sim = m3b.simulate(np.random.default_rng(21), sc, 20000)
            d1, d2 = m3b.data(sim, 1), m3b.data(sim, 2)
            # Experiment-level means are independent across replicates.
            for x, target in ((d1.a, sc.rates[0].mean()), (d1.b, sc.rates[1].mean()),
                              (d2.a, sc.e2[0]), (d2.b, sc.e2[1])):
                per = x.mean((1, 2))
                se = per.std(ddof=1) / np.sqrt(len(per))
                self.assertLess(abs(per.mean() - target), 4 * se + 1e-4, fam)

    def test_exclusive_and_zero_e2(self):
        sc = m3b.Scenario(family='G2', mix_pi=0.05, p_a=0.06, e2_a=0.0, sigma=0.5, quartets=4)
        sim = m3b.simulate(np.random.default_rng(3), sc, 500)
        self.assertTrue(set(np.unique(sim['outcome'])) <= {0, 1})
        self.assertEqual(m3b.data(sim, 2).a.sum(), 0)

    def test_infeasible(self):
        with self.assertRaises(ValueError):
            m3b.Scenario(p_a=0.02, rd=-0.03)
        with self.assertRaises(ValueError):
            m3b.Scenario(e2_a=0.05, rd2=-0.1)


class Schedules(unittest.TestCase):
    @staticmethod
    def exact_cap(Q, b, cap):
        q = 1 - (1 - b) ** 4
        tot = 0.0
        for k in range(cap + 1):
            pk = comb(Q, k) * q ** k * (1 - q) ** (Q - k)
            pr = sum(comb(k + f - 1, f) * (1 - q) ** k * q ** f for f in range(cap - k + 1)) if k else 1.0
            tot += pk * pr
        return tot

    def test_no_invalidation_is_identity(self):
        sc = m3b.Scenario(quartets=4)
        starts, orient, valid, run, _ = m3b.plan(np.random.default_rng(1), sc, 50)
        self.assertTrue(valid.all() and (run == 16).all())
        np.testing.assert_array_equal(starts[0], [0, 4, 8, 12])

    def test_completion_probabilities(self):
        Q, b, reps = 8, 0.05, 6000
        q = 1 - (1 - b) ** 4
        expect = {('cap', 2): self.exact_cap(Q, b, 2), ('immediate', 2): self.exact_cap(Q, b, 2),
                  ('cap', 6): self.exact_cap(Q, b, 6), ('restart', 3): 1 - (1 - (1 - q) ** Q) ** 3}
        for (kind, cap), p in expect.items():
            sc = m3b.Scenario(quartets=Q, invalid=('indep', b), policy=(kind, cap))
            _, _, valid, run, _ = m3b.plan(np.random.default_rng(7), sc, reps)
            se = np.sqrt(p * (1 - p) / reps)
            self.assertLess(abs(valid.mean() - p), 4 * se + 1e-3, (kind, cap))

    def test_positions_distinct_and_ordered(self):
        for kind, cap in (('cap', 2), ('immediate', 2), ('restart', 3)):
            sc = m3b.Scenario(quartets=6, invalid=('indep', 0.05), policy=(kind, cap))
            starts, _, valid, run, _ = m3b.plan(np.random.default_rng(9), sc, 400)
            for r in np.flatnonzero(valid):
                self.assertEqual(len(set(starts[r])), 6)
                self.assertTrue((starts[r] % 4 == 0).all() and (starts[r] < run[r]).all())
                if kind != 'cap':
                    self.assertTrue((np.diff(starts[r]) > 0).all())


class R1(unittest.TestCase):
    def test_matches_scalar_reference(self):
        rng = np.random.default_rng(4)
        for x in (rng.random(12) ** 4, np.zeros(16), (rng.random(24) < 0.1) * 1.0, rng.random(8)):
            lo, up = m3b._wsr_bounds(x[None, :], 0.025)
            rlo, rup = wsr_reference(list(x), 0.025)
            self.assertAlmostEqual(lo[0], rlo)
            self.assertAlmostEqual(up[0], rup)

    def test_coverage_on_iid_bounded_data(self):
        rng = np.random.default_rng(5)
        for gen, mean in ((lambda s: (rng.random(s) < 0.1) * 1.0, 0.1),
                          (lambda s: rng.beta(0.2, 1.8, s), 0.1)):
            x = gen((4000, 20))
            lo, up = m3b._wsr_bounds(x, 0.05)
            for covered in ((up >= mean).sum(), (lo <= mean).sum()):
                cp_up = 1.0 if covered == 4000 else stats.beta.ppf(0.95, covered + 1, 4000 - covered)
                self.assertGreaterEqual(cp_up, 0.95)            # not demonstrably under 95%

    def test_all_zero_floor(self):
        for n in (8, 16, 40):
            _, up = m3b._wsr_bounds(np.zeros((1, n)), 0.025)
            self.assertGreaterEqual(up[0], 1 - 0.025 ** (1 / n) - 1e-9)

    def test_bernstein_formula(self):
        a = np.array([[0, 0, 0.2, 0, 0.1, 0, 0, 0.4]])
        b = np.array([[0, 0.1, 0, 0, 0, 0.3, 0, 0]])
        d = type('D', (), {'block_a': a, 'block_b': b, 'reps': 1})()
        ru, rl, _ = m3b.r1_bernstein(d, [0.5, 2.0])
        n = 8
        r = lambda x: np.sqrt(2 * x.var(ddof=1) * np.log(80) / n) + 7 * np.log(80) / (3 * (n - 1))
        u = b.mean() + r(b[0]) - (a.mean() - r(a[0]))
        self.assertEqual(list(ru[0]), [u < 0.5, u < 2.0])


class R2R3(unittest.TestCase):
    def test_bootstrap_matches_loop(self):
        sim = m3b.simulate(np.random.default_rng(6), m3b.Scenario(p_a=0.06, sigma=1.0, quartets=4), 3)
        d = m3b.data(sim, 1)
        boot = 74
        ru, rl, st = m3b.r2_bootstrap_t(d, [0.0, 0.05, 0.1], rng=np.random.default_rng(99), boot=boot, batch=37)
        rng = np.random.default_rng(99)
        n = d.nb
        ts = [[] for _ in range(3)]
        for s in range(0, boot, 37):
            ia = rng.integers(0, n, (3, 37, n))
            ib = rng.integers(0, n, (3, 37, n))
            for r in range(3):
                for j in range(37):
                    sa, sb = d.block_a[r][ia[r, j]], d.block_b[r][ib[r, j]]
                    dd = sb.mean() - sa.mean() - (d.block_b[r].mean() - d.block_a[r].mean())
                    ss = np.sqrt(sa.var(ddof=1) / n + sb.var(ddof=1) / n)
                    ts[r].append(dd / ss if ss > 0 else np.sign(dd) * np.inf)
        for r in range(3):
            t = np.nan_to_num(np.array(ts[r]), nan=0.0, posinf=np.inf, neginf=-np.inf)
            est_d = d.block_b[r].mean() - d.block_a[r].mean()
            se = np.sqrt(d.block_a[r].var(ddof=1) / n + d.block_b[r].var(ddof=1) / n)
            if se == 0:
                continue
            ql = np.quantile(t, 0.05, method='lower')
            up = est_d - ql * se if np.isfinite(ql) else np.inf
            self.assertEqual(list(ru[r]), [up < x for x in (0.0, 0.05, 0.1)])

    def test_r3_flat_differences_do_not_reject(self):
        d = m3b.data(m3b.simulate(np.random.default_rng(8), m3b.Scenario(p_a=0.04, quartets=6), 1), 1)
        d.quartet_d[:] = 0.0
        ru, rl, _ = m3b.r3_randomisation(d, [0.0])
        self.assertFalse(ru.any() or rl.any())

    def test_r3_cannot_reject_with_four_quartets(self):
        sim = m3b.simulate(np.random.default_rng(8), m3b.Scenario(p_a=0.04, rd=-0.02, quartets=4), 500)
        ru, rl, _ = m3b.call('R3', m3b.data(sim, 1), [0.075, 0.0])
        self.assertFalse(ru.any() or rl.any())

    def test_r3_matches_enumeration(self):
        sim = m3b.simulate(np.random.default_rng(10), m3b.Scenario(p_a=0.06, sigma=0.5, quartets=6), 4)
        d = m3b.data(sim, 1)
        ru, _, _ = m3b.r3_randomisation(d, [0.075])
        pats = design.balanced_patterns(6)
        for r in range(4):
            z = d.quartet_d[r] - 0.075
            def t(v):
                if v.std(ddof=1) > 0:
                    return v.mean() / (v.std(ddof=1) / np.sqrt(6))
                return np.inf if v.mean() > 0 else (-np.inf if v.mean() < 0 else 0.0)
            tobs = t(z)
            p = np.mean([t(np.where(pt == d.orient[r], 1, -1) * z) <= tobs + 1e-12 for pt in pats])
            self.assertEqual(bool(ru[r, 0]), p <= 0.05)


class Common(unittest.TestCase):
    def test_degenerate_rule_applies_to_new_methods(self):
        sim = m3b.simulate(np.random.default_rng(11), m3b.Scenario(p_a=0.04, quartets=4), 10)
        sim['outcome'][:] = 0
        d = m3b.data(sim, 1)
        for m in ('R1', 'R1EB', 'R2', 'R3'):
            ru, rl, st = m3b.call(m, d, [0.0, 0.075], rng=np.random.default_rng(1))
            self.assertTrue((st == est.DEGENERATE).all(), m)
            self.assertFalse(ru.any() or rl.any(), m)

    def test_floor(self):
        self.assertEqual([m3b.floor_blocks(x) for x in (0.05, 0.075, 0.10)], [59, 39, 29])
        n = m3b.floor_blocks(0.075) - 1
        self.assertGreater((1 - 0.075) ** n, 0.05)          # all-zero data still likely under H0

    def test_hierarchy(self):
        h = m3b.hierarchy
        self.assertEqual(h(True, False, True, False), 'LIVENESS_NONINFERIOR')
        self.assertEqual(h(True, False, False, False), 'LIVENESS_INCONCLUSIVE')
        self.assertEqual(h(True, False, False, True), 'LIVENESS_INFERIOR')
        self.assertEqual(h(False, False, True, False), 'LIVENESS_INCONCLUSIVE')   # E2 cannot compensate
        self.assertEqual(h(False, True, True, False), 'LIVENESS_INFERIOR')


if __name__ == '__main__':
    unittest.main()
