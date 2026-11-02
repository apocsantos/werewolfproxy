"""Synthetic S3-P2 TCP-hop outcomes (spec section 6.1).

For block b of arm X, seed position s and run r (campaign position t_b):

    logit P(E1) = mu[X, s] + tau * g(t_b) + u_b + w[b, r]

- u_b ~ N(0, sigma_X^2): the block's persistent sandbox state; sigma may
  differ by arm (asymmetric dependence).
- w[b, r]: within-block carry-over, a stationary AR(1) over the run sequence
  with persistence phi and standard deviation SIGMA_W (0 when phi = 0).
- g(t): campaign drift; none, linear (t - 1/2), or a step of 1 at t >= 1/2.

mu[X, s] is calibrated so that the target probabilities hold marginally,
integrated over u, w and the design's campaign positions, by Gauss-Hermite
quadrature: a logit-scale difference is not the risk difference.

Each leg's outcome: 1 = E1 (deadline event), 2 = E2 (clean early end, with
probability Q_E2 among legs that are not E1), 0 = completed. E1 and E2 are
mutually exclusive by construction.
"""
import numpy as np
from numpy.polynomial.hermite_e import hermegauss
from scipy.optimize import brentq
from scipy.special import expit, logit

from . import design

SIGMA_W = 1.0            # carry-over component's SD when phi > 0 (modelling choice)
Q_E2 = 0.12              # E2 among non-E1 legs (Stage36-37: about 12%)
_GH_X, _GH_W = hermegauss(64)
_GH_W = _GH_W / _GH_W.sum()


def drift_shape(kind, t):
    t = np.asarray(t, dtype=float)
    if kind == 'none':
        return np.zeros_like(t)
    if kind == 'linear':
        return t - 0.5
    if kind == 'step':
        return (t >= 0.5).astype(float)
    raise ValueError(kind)


def marginal(mu, sd, shifts):
    """E[expit(mu + shift + z)], z ~ N(0, sd^2), averaged over shifts."""
    shifts = np.atleast_1d(shifts)
    z = mu + shifts[:, None] + sd * _GH_X[None, :]
    return float((expit(z) @ _GH_W).mean())


def calibrate(target, sd, shifts):
    if not 0 < target < 1:
        raise ValueError(f'target probability {target} outside (0, 1)')
    lo, hi = logit(target) - 30, logit(target) + 30
    return brentq(lambda m: marginal(m, sd, shifts) - target, lo, hi, xtol=1e-12)


def seed_rates(p, pattern):
    """Seed-position rates with mean p: equal, or seed 2030 at 3x the others."""
    if pattern == 'equal':
        return np.full(3, p)
    if pattern == 'seed2030x3':
        return p * np.array([1.0, 1.0, 3.0]) * 3.0 / 5.0
    raise ValueError(pattern)


class Scenario:
    """One cell of the grid. Probabilities and RD are fractions (0.04, 0.075)."""

    def __init__(self, p_a, rd, sigma, quartets, asym=0.0, phi=0.0, seeds='equal',
                 drift='none', tau=0.0, p_invalid=0.0, sigma_b=None):
        self.p_a, self.rd, self.sigma, self.quartets = p_a, rd, sigma, quartets
        self.asym, self.phi, self.seeds = asym, phi, seeds
        self.drift, self.tau, self.p_invalid = drift, tau, p_invalid
        self.sigma_b = sigma + asym if sigma_b is None else sigma_b
        self.rates = {0: seed_rates(p_a, seeds), 1: seed_rates(p_a, seeds) + rd}
        if (self.rates[1] <= 0).any() or (self.rates[1] >= 1).any():
            raise ValueError('infeasible scenario: a candidate seed rate outside (0, 1)')
        self.sd_w = SIGMA_W if phi > 0 else 0.0
        nominal_t = np.arange(4 * quartets) / max(4 * quartets - 1, 1)
        shifts = tau * drift_shape(drift, nominal_t)
        self.mu = {}
        for arm, sd_u in ((0, sigma), (1, self.sigma_b)):
            sd = np.hypot(sd_u, self.sd_w)
            self.mu[arm] = np.array([calibrate(q, sd, shifts) for q in self.rates[arm]])

    @property
    def true_rd(self):
        return float((self.rates[1] - self.rates[0]).mean())

    def as_dict(self):
        return {k: getattr(self, k) for k in ('p_a', 'rd', 'sigma', 'sigma_b', 'quartets', 'asym',
                                               'phi', 'seeds', 'drift', 'tau', 'p_invalid')}


def simulate(rng, sc, reps):
    """reps experiments of scenario sc.

    Returns a dict:
      outcome   (reps, Q, 4, 15) int8   analysed quartets in campaign order
      arm       (reps, Q, 4)     int8   0 = A, 1 = B
      orient    (reps, Q)        int8
      valid     (reps,)          bool   False: replacement cap exceeded
      replaced  (reps,)          int8   replacement quartets used
    """
    Q = sc.quartets
    orient_all, analysed, valid, used = design.schedule(rng, reps, Q, sc.p_invalid)
    slots = orient_all.shape[1]
    arm_all = design.arms_of(orient_all)                         # (reps, slots, 4)
    t = np.arange(slots * 4).reshape(slots, 4) / max(4 * Q - 1, 1)
    shift = sc.tau * drift_shape(sc.drift, t)                     # (slots, 4)
    sd_u = np.where(arm_all == 1, sc.sigma_b, sc.sigma)
    u = rng.standard_normal((reps, slots, 4)) * sd_u
    lin = np.where(arm_all[..., None] == 1, sc.mu[1][design.SEED_POS], sc.mu[0][design.SEED_POS])
    lin = lin + shift[None, :, :, None] + u[..., None]
    if sc.sd_w > 0:
        eps = rng.standard_normal((reps, slots, 4, design.RUNS_PER_BLOCK))
        w = np.empty_like(eps)
        w[..., 0] = eps[..., 0]
        innov = np.sqrt(1 - sc.phi ** 2)
        for r in range(1, design.RUNS_PER_BLOCK):
            w[..., r] = sc.phi * w[..., r - 1] + innov * eps[..., r]
        lin = lin + sc.sd_w * w
    p1 = expit(lin)
    draw = rng.random(p1.shape)
    outcome = np.where(draw < p1, 1, np.where(draw < p1 + Q_E2 * (1 - p1), 2, 0)).astype(np.int8)
    idx = analysed[:, :, None, None]
    return {
        'outcome': np.take_along_axis(outcome, np.broadcast_to(idx, (reps, Q, 4, design.RUNS_PER_BLOCK)), axis=1),
        'arm': np.take_along_axis(arm_all, analysed[:, :, None].repeat(4, axis=2), axis=1).astype(np.int8),
        'orient': np.take_along_axis(orient_all, analysed, axis=1),
        'valid': valid,
        'replaced': used,
    }
