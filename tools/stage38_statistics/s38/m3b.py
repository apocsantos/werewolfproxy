"""Stage38 M3b: generator families G1-G3, the E2 endpoint, replacement
policies, and the new estimator families R1-R3.

The M3 modules are unchanged; this module reuses their design, Data and
the common degenerate-case rule.

Generator (per block b at campaign position t, arm X, seed s, run r):

    E1:  logit P = mu[X, s] + tau*g(t) + B_b + w[b, r]
    E2 (given not E1):  logit P = nu[X, s] + lam2 * B_b

B_b is the block's latent state:
  G1      u_b ~ N(0, sigma_X^2)                                  (M3)
  G2      u_b + J_b * MIX_SHIFT, J_b ~ Bernoulli(pi)             (very bad blocks)
  G3ar    u_b + e_t, e an AR(1) host environment over consecutive
          blocks in campaign order (both arms share it), stationary SD
          ENV_SD, persistence rho
  G3step  u_b + STEP_SHIFT * 1[t >= T], T ~ U(0, 1) per experiment
          (abrupt deterioration at a random campaign point)

mu and nu are calibrated so that the target E1 and E2 rates hold
marginally, by Gauss-Hermite quadrature over every normal component of
B, w and the campaign positions.
"""
from math import comb

import numpy as np
from numpy.polynomial.hermite_e import hermegauss
from scipy import stats
from scipy.optimize import brentq
from scipy.special import expit, logit

from . import design, estimators as est, generator as g1

MIX_SHIFT = 3.0
STEP_SHIFT = 1.0
ENV_SD = 1.0
_X, _W = hermegauss(48)
_W = _W / _W.sum()


def components(family, sigma, t, mix_pi=0.0, env_sd=ENV_SD):
    """Normal mixture [(weight, mean, sd)] of the block latent at position t."""
    if family == 'G1':
        return [(1.0, 0.0, sigma)]
    if family == 'G2':
        return [(1 - mix_pi, 0.0, sigma), (mix_pi, MIX_SHIFT, sigma)]
    if family == 'G3ar':
        return [(1.0, 0.0, float(np.hypot(sigma, env_sd)))]
    if family == 'G3step':
        return [(1 - t, 0.0, sigma), (t, STEP_SHIFT, sigma)]
    raise ValueError(family)


def _rates(mu, nu, lam2, comps, shift, sd_w):
    """Marginal (P(E1), P(E2)) for one position: E over B (mixture) and w."""
    e1 = e2 = 0.0
    for wgt, m, sd in comps:
        if wgt == 0:
            continue
        b = m + sd * _X                                         # block latent nodes
        p1w = expit(mu + shift + b[:, None] + sd_w * _X[None, :]) @ _W  # E_w P(E1 | b)
        q2 = expit(nu + lam2 * b)
        e1 += wgt * (p1w @ _W)
        e2 += wgt * ((q2 * (1 - p1w)) @ _W)
    return e1, e2


class Scenario:
    """A cell of the M3b grids. Fractions throughout."""

    def __init__(self, family='G1', p_a=0.04, rd=0.0, e2_a=0.12, rd2=0.0, lam2=0.0, sigma=0.0,
                 asym=0.0, phi=0.0, seeds='equal', drift='none', tau=0.0, quartets=8,
                 mix_pi=0.0, env_rho=0.0, invalid=None, policy=None):
        self.family, self.p_a, self.rd, self.e2_a, self.rd2 = family, p_a, rd, e2_a, rd2
        self.lam2, self.sigma, self.asym, self.phi = lam2, sigma, asym, phi
        self.seeds, self.drift, self.tau, self.quartets = seeds, drift, tau, quartets
        self.mix_pi, self.env_rho = mix_pi, env_rho
        self.invalid, self.policy = invalid, policy
        self.sigma_b = sigma + asym
        self.sd_w = g1.SIGMA_W if phi > 0 else 0.0
        self.rates = {0: g1.seed_rates(p_a, seeds), 1: g1.seed_rates(p_a, seeds) + rd}
        self.e2 = {0: e2_a, 1: e2_a + rd2}
        for arm in (0, 1):
            if (self.rates[arm] <= 0).any() or (self.rates[arm] >= 1).any():
                raise ValueError('infeasible scenario: an E1 seed rate outside (0, 1)')
            if not 0 <= self.e2[arm] < 1:
                raise ValueError('infeasible scenario: E2 rate outside [0, 1)')
            if (self.rates[arm] + self.e2[arm] >= 1).any():
                raise ValueError('infeasible scenario: E1 + E2 >= 1')
        nominal = np.arange(4 * quartets) / max(4 * quartets - 1, 1)
        self.positions = nominal
        self.shifts = tau * g1.drift_shape(drift, nominal)
        self.mu, self.nu = {}, {}
        for arm, sd_u in ((0, sigma), (1, self.sigma_b)):
            comps_t = [components(family, sd_u, t, mix_pi) for t in nominal]
            mus = []
            for q in self.rates[arm]:
                f = lambda m: np.mean([_rates(m, 0.0, 0.0, c, s, self.sd_w)[0]
                                       for c, s in zip(comps_t, self.shifts)]) - q
                mus.append(brentq(f, logit(q) - 30, logit(q) + 30, xtol=1e-12))
            self.mu[arm] = np.array(mus)
            nus = []
            for m in self.mu[arm]:
                if self.e2[arm] == 0:
                    nus.append(-np.inf)
                    continue
                f = lambda n: np.mean([_rates(m, n, lam2, c, s, self.sd_w)[1]
                                       for c, s in zip(comps_t, self.shifts)]) - self.e2[arm]
                nus.append(brentq(f, -40, 40, xtol=1e-12))
            self.nu[arm] = np.array(nus)

    @property
    def true_rd(self):
        return float((self.rates[1] - self.rates[0]).mean())

    @property
    def true_rd2(self):
        return float(self.e2[1] - self.e2[0])

    def marginal(self, arm, seed):
        comps_t = [components(self.family, self.sigma if arm == 0 else self.sigma_b, t, self.mix_pi)
                   for t in self.positions]
        vals = [_rates(self.mu[arm][seed], self.nu[arm][seed], self.lam2, c, s, self.sd_w)
                for c, s in zip(comps_t, self.shifts)]
        return tuple(np.mean(vals, axis=0))


# --- schedules: invalidation and replacement policies ------------------------

def _invalid_prob(model, b, t, env):
    """Per-block invalidation probability at campaign fraction t."""
    if model == 'indep':
        return np.full_like(t, b, dtype=float)
    if model == 'late':                                   # rises with time, mean b over [0, 1]
        return np.clip(2 * b * t, 0, 1)
    if model == 'host':                                   # driven by the host environment
        return expit(logit(b) + 1.5 * env)
    raise ValueError(model)


def plan(rng, sc, reps):
    """Executed campaign per replicate, under sc.invalid = (model, b) and
    sc.policy = (kind, cap). Returns analysed quartet start positions
    (reps, Q), their orientations (reps, Q), validity (reps,) and blocks
    run (reps,). Positions are block indices in execution order."""
    Q = sc.quartets
    orient = design.balanced_orientations(rng, reps, Q)
    if not sc.invalid:
        starts = np.tile(np.arange(Q) * 4, (reps, 1))
        return starts, orient, np.ones(reps, bool), np.full(reps, 4 * Q), None
    model, b = sc.invalid
    kind, cap = sc.policy
    horizon = 4 * Q * 4 + 64                                # enough positions for any policy
    env = _env_series(rng, sc, reps, horizon) if sc.family == 'G3ar' else np.zeros((reps, horizon))
    t = np.arange(horizon) / max(4 * Q - 1, 1)
    u = rng.random((reps, horizon))
    if model == 'quartet':                                  # whole quartet at once, rate b per quartet
        quartet_bad = rng.random((reps, horizon // 4)) < b
    starts = np.zeros((reps, Q), np.int64)
    valid = np.ones(reps, bool)
    run = np.zeros(reps, np.int64)
    for r in range(reps):
        if model == 'quartet':
            bad_at = lambda pos: bool(quartet_bad[r, pos // 4])
        else:
            pb = _invalid_prob(model, b, t, env[r])
            bad_at = lambda pos: bool((u[r, pos:pos + 4] < pb[pos:pos + 4]).any())
        pos = 0
        if kind == 'cap':                     # V5: planned quartets, then replacements at the end
            failed = []
            for q in range(Q):
                if bad_at(pos):
                    failed.append(q)
                else:
                    starts[r, q] = pos
                pos += 4
            used = 0
            for q in failed:
                while valid[r]:
                    if used >= cap:
                        valid[r] = False
                        break
                    used += 1
                    ok = not bad_at(pos)
                    if ok:
                        starts[r, q] = pos
                    pos += 4
                    if ok:
                        break
                if not valid[r]:
                    break
        elif kind == 'immediate':             # the replacement runs right after the invalid quartet
            used = 0
            for q in range(Q):
                while True:
                    if not bad_at(pos):
                        starts[r, q] = pos
                        pos += 4
                        break
                    pos += 4
                    if used >= cap:
                        valid[r] = False
                        break
                    used += 1
                if not valid[r]:
                    break
        elif kind == 'restart':               # abandon at the first invalid quartet and rerun
            attempts = 0
            while True:
                attempts += 1
                ok = True
                for q in range(Q):
                    if bad_at(pos):
                        ok = False
                        pos += 4
                        break
                    starts[r, q] = pos
                    pos += 4
                if ok:
                    break
                if attempts >= cap:
                    valid[r] = False
                    break
        else:
            raise ValueError(kind)
        run[r] = pos
    return starts, orient, valid, run, env


def _env_series(rng, sc, reps, horizon):
    """AR(1) host environment over block positions, stationary SD ENV_SD."""
    rho = sc.env_rho
    e = np.empty((reps, horizon))
    e[:, 0] = rng.standard_normal(reps) * ENV_SD
    innov = np.sqrt(1 - rho ** 2) * ENV_SD
    for k in range(1, horizon):
        e[:, k] = rho * e[:, k - 1] + innov * rng.standard_normal(reps)
    return e


def simulate(rng, sc, reps):
    """Outcomes for the analysed quartets: the M3 simulate() layout plus
    'run' (blocks executed). Outcome codes: 1 = E1, 2 = E2, 0 = completed."""
    Q = sc.quartets
    starts, orient, valid, run, env = plan(rng, sc, reps)
    arm = design.arms_of(orient)                                         # (R, Q, 4)
    pos = starts[:, :, None] + np.arange(4)[None, None, :]               # block positions
    t = pos / max(4 * Q - 1, 1)
    sd_u = np.where(arm == 1, sc.sigma_b, sc.sigma)
    latent = rng.standard_normal((reps, Q, 4)) * sd_u
    if sc.family == 'G2':
        latent = latent + MIX_SHIFT * (rng.random((reps, Q, 4)) < sc.mix_pi)
    elif sc.family == 'G3ar':
        if env is None:
            env = _env_series(rng, sc, reps, int(pos.max()) + 1)
        latent = latent + np.take_along_axis(env, pos.reshape(reps, -1), axis=1).reshape(reps, Q, 4)
    elif sc.family == 'G3step':
        cut = rng.random(reps)
        latent = latent + STEP_SHIFT * (t >= cut[:, None, None])
    shift = sc.tau * g1.drift_shape(sc.drift, t)
    seed = design.SEED_POS
    mu = np.where(arm[..., None] == 1, sc.mu[1][seed], sc.mu[0][seed])  # (R, Q, 4, 15)
    nu = np.where(arm[..., None] == 1, sc.nu[1][seed], sc.nu[0][seed])
    lin = mu + (shift + latent)[..., None]
    if sc.sd_w > 0:
        eps = rng.standard_normal(lin.shape)
        w = np.empty_like(eps)
        w[..., 0] = eps[..., 0]
        for r in range(1, design.RUNS_PER_BLOCK):
            w[..., r] = sc.phi * w[..., r - 1] + np.sqrt(1 - sc.phi ** 2) * eps[..., r]
        lin = lin + sc.sd_w * w
    p1 = expit(lin)
    with np.errstate(over='ignore'):
        q2 = expit(nu + sc.lam2 * latent[..., None])
    draw = rng.random(p1.shape)
    outcome = np.where(draw < p1, 1, np.where(draw < p1 + q2 * (1 - p1), 2, 0)).astype(np.int8)
    return {'outcome': outcome, 'arm': arm.astype(np.int8), 'orient': orient.astype(np.int8),
            'valid': valid, 'replaced': ((run - 4 * Q) // 4).astype(np.int16), 'run': run}


# --- endpoint-aware data ------------------------------------------------------

def data(sim, endpoint):
    """estimators.Data for E1 (code 1) or E2 (code 2); C0-C7 unchanged."""
    view = dict(sim)
    view['outcome'] = np.where(sim['outcome'] == endpoint, 1, 0).astype(np.int8)
    return est.Data(view)


# --- R1: bounded block-level inference ---------------------------------------

R1_GRID = np.linspace(0.0, 1.0, 401)                        # step 0.0025


def _wsr_bounds(x, alpha, c=0.75):
    """Waudby-Smith & Ramdas (2023) betting bounds for the mean of
    independent [0, 1] observations with a common mean, fixed n, predictable
    plug-in bets. x: (R, n) in campaign order. Returns (lower, upper), each
    one-sided at level alpha, rounded outward to the grid."""
    R, n = x.shape
    m = R1_GRID[None, :, None]                                          # (1, G, 1)
    csum = np.cumsum(x, 1)
    i = np.arange(1, n + 1)
    mu_hat = (0.5 + csum) / (i + 1)                                     # after step i
    dev = np.cumsum((x - mu_hat) ** 2, 1)
    var_hat = (0.25 + dev) / (i + 1)
    var_prev = np.concatenate([np.full((R, 1), 0.25), var_hat[:, :-1]], 1)   # predictable
    lam = np.sqrt(2 * np.log(1 / alpha) / (n * var_prev))[:, None, :]       # (R, 1, n)
    xx = x[:, None, :]
    with np.errstate(divide='ignore', invalid='ignore'):
        lam_up = np.minimum(lam, c / np.maximum(1 - m, 1e-12))         # bets X below m
        k_up = np.prod(1 + lam_up * (m - xx), axis=2)                  # (R, G)
        lam_lo = np.minimum(lam, c / np.maximum(m, 1e-12))             # bets X above m
        k_lo = np.prod(1 + lam_lo * (xx - m), axis=2)
    thr = 1 / alpha
    rej_up = k_up >= thr                                                # m above the mean
    rej_lo = k_lo >= thr                                                # m below the mean
    G = R1_GRID
    upper = np.where(rej_up.any(1), G[np.argmax(rej_up, 1)], 1.0)
    lower = np.where(rej_lo.any(1), G[len(G) - 1 - np.argmax(rej_lo[:, ::-1], 1)], 0.0)
    return lower, upper


def r1_betting(d, thresholds, alpha=0.05, rng=None):
    """U = U_B - L_A, L = L_B - U_A with alpha/2 per arm (Bonferroni).
    Units: block proportions in campaign order, one arm at a time."""
    la, ua = _wsr_bounds(d.block_a, alpha / 2)
    lb, ub = _wsr_bounds(d.block_b, alpha / 2)
    upper, lower = ub - la, lb - ua
    th = np.asarray(thresholds, float)
    return (upper[:, None] < th[None, :], lower[:, None] > th[None, :],
            np.full(d.reps, est.OK, np.int8))


def r1_bernstein(d, thresholds, alpha=0.05, rng=None):
    """Empirical Bernstein (Maurer & Pontil 2009, theorem 4), per arm at
    alpha/2, Bonferroni-combined. Valid for i.i.d. [0, 1] block units."""
    def bound(x, a):
        n = x.shape[1]
        v = x.var(1, ddof=1)
        r = np.sqrt(2 * v * np.log(2 / a) / n) + 7 * np.log(2 / a) / (3 * (n - 1))
        return x.mean(1) - r, x.mean(1) + r
    la, ua = bound(d.block_a, alpha / 2)
    lb, ub = bound(d.block_b, alpha / 2)
    th = np.asarray(thresholds, float)
    return ((ub - la)[:, None] < th[None, :], (lb - ua)[:, None] > th[None, :],
            np.full(d.reps, est.OK, np.int8))


# --- R2: block bootstrap-t ----------------------------------------------------

R2_BOOT = 999


def r2_bootstrap_t(d, thresholds, alpha=0.05, rng=None, boot=R2_BOOT, batch=37):
    """Studentised bootstrap: blocks resampled with replacement within arm;
    t* = (d* - d) / se*. A resample with se* = 0 gets t* = +inf, -inf or 0 by
    the sign of d* - d (so a lower-tail infinity makes U = +inf). U = d -
    t*_(alpha) se, L = d - t*_(1-alpha) se (Hall's bootstrap-t)."""
    a, b = d.block_a, d.block_b
    R, n = a.shape
    est_d = b.mean(1) - a.mean(1)
    se = np.sqrt(a.var(1, ddof=1) / n + b.var(1, ddof=1) / n)
    tstar = np.empty((R, boot))
    for s in range(0, boot, batch):
        k = min(batch, boot - s)
        ia = rng.integers(0, n, (R, k, n))
        ib = rng.integers(0, n, (R, k, n))
        sa = np.take_along_axis(a[:, None, :].repeat(k, 1), ia, 2)
        sb = np.take_along_axis(b[:, None, :].repeat(k, 1), ib, 2)
        dd = sb.mean(2) - sa.mean(2) - est_d[:, None]
        ss = np.sqrt(sa.var(2, ddof=1) / n + sb.var(2, ddof=1) / n)
        with np.errstate(divide='ignore', invalid='ignore'):
            tstar[:, s:s + k] = np.where(ss > 0, dd / ss, np.sign(dd) * np.inf)
    tstar = np.nan_to_num(tstar, nan=0.0, posinf=np.inf, neginf=-np.inf)
    q_lo = np.quantile(tstar, alpha, axis=1, method='lower')
    q_hi = np.quantile(tstar, 1 - alpha, axis=1, method='higher')
    with np.errstate(invalid='ignore'):
        upper = np.where(np.isfinite(q_lo), est_d - q_lo * se, np.inf)
        lower = np.where(np.isfinite(q_hi), est_d - q_hi * se, -np.inf)
    ok = se > 0
    th = np.asarray(thresholds, float)
    status = np.where(ok, est.OK, est.DEGENERATE).astype(np.int8)
    return (ok[:, None] & (upper[:, None] < th[None, :]),
            ok[:, None] & (lower[:, None] > th[None, :]), status)


# --- R3: studentised randomisation over the balanced design ------------------

R3_MAX_PATTERNS = 13000


def r3_randomisation(d, thresholds, alpha=0.05, rng=None):
    """Studentised sign-flip randomisation test over every balanced
    orientation pattern (Wu & Ding 2021 style: asymptotically valid for the
    weak null of an average effect; exact only under a sharp null)."""
    q = d.quartets
    k = len(thresholds)
    if q < 2 or comb(q, q // 2) > R3_MAX_PATTERNS:
        z = np.zeros((d.reps, k), bool)
        return z, z.copy(), np.full(d.reps, est.NA, np.int8)
    pats = design.balanced_patterns(q)
    batch = max(1, 4_000_000 // (len(pats) * q))
    rej_u = np.zeros((d.reps, k), bool)
    rej_l = np.zeros((d.reps, k), bool)
    status = np.full(d.reps, est.OK, np.int8)
    for s in range(0, d.reps, batch):
        e = slice(s, s + batch)
        signs = np.where(pats[None] == d.orient[e][:, None, :], 1.0, -1.0)   # (r, P, Q)
        for j, d0 in enumerate(thresholds):
            z = d.quartet_d[e] - d0
            def tstat(v):
                sd = v.std(-1, ddof=1)
                mean = v.mean(-1)
                # Zero spread: +-inf by the sign of the mean, 0 when the mean is 0
                # (sign(0) * inf would be NaN, which compares false and rejects).
                flat = np.where(mean > 0, np.inf, np.where(mean < 0, -np.inf, 0.0))
                with np.errstate(divide='ignore', invalid='ignore'):
                    return np.where(sd > 0, mean / (sd / np.sqrt(q)), flat)
            t_obs = tstat(z)
            t_ref = tstat(signs * z[:, None, :])
            eps = 1e-12
            rej_u[e, j] = (t_ref <= t_obs[:, None] + eps).mean(1) <= alpha
            rej_l[e, j] = (t_ref >= t_obs[:, None] - eps).mean(1) <= alpha
    return rej_u, rej_l, status


NEW_METHODS = {'R1': r1_betting, 'R1EB': r1_bernstein, 'R2': r2_bootstrap_t, 'R3': r3_randomisation}


def call(method, d, thresholds, rng=None, alpha=0.05):
    """Any C0-C7 or R method, under the common degenerate-case rule."""
    if method in NEW_METHODS:
        f = NEW_METHODS[method]
        inner = lambda dd, th, alpha=alpha: f(dd, th, alpha=alpha, rng=rng)
        return est._uniform_degeneracy(inner)(d, thresholds, alpha=alpha)
    return est.METHODS[method](d, thresholds, alpha=alpha)


# --- the all-or-nothing floor (Phase A) ---------------------------------------

def floor_blocks(delta, alpha=0.05):
    """Smallest number of candidate blocks n with (1 - delta)^n <= alpha:
    below it, no test valid for arbitrary within-block dependence can
    declare non-inferiority at margin delta, even on all-zero data."""
    return int(np.ceil(np.log(alpha) / np.log1p(-delta)))


# --- E1 then E2 fixed-sequence hierarchy --------------------------------------

def hierarchy(e1_ni, e1_inf, e2_ni, e2_inf):
    """Overall comparative finding (frozen plan section 5): E2 is tested only
    after E1 is non-inferior; E2 never compensates for E1."""
    if e1_inf:
        return 'LIVENESS_INFERIOR'
    if not e1_ni:
        return 'LIVENESS_INCONCLUSIVE'
    if e2_inf:
        return 'LIVENESS_INFERIOR'
    if not e2_ni:
        return 'LIVENESS_INCONCLUSIVE'
    return 'LIVENESS_NONINFERIOR'
