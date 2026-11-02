"""Candidate procedures C0-C7 (spec section 5). Research candidates only.

Every procedure answers, for each threshold d0 in `thresholds`, two one-sided
questions at level alpha (one-sided):

    rej_u[:, k]  the upper bound U < d0   (reject H0: RD >= d0)
    rej_l[:, k]  the lower bound L > d0   (reject H0: RD <= d0)

and a status per replicate:

    OK          ordinary computation
    DEGENERATE  the estimator's variance is zero or undefined (for example no
                events at all); defined behaviour: no rejection in either
                direction, never a silent substitute interval
    FAILED      a numerical failure; counted, never treated as success
    NA          not computable by design (C5 with too many patterns)

A degenerate or failed replicate never rejects.

All procedures are vectorised over replicates. Where a closed form exists it
is used; C4's CR2 matrices depend only on the design and are precomputed.
"""
from functools import lru_cache

import numpy as np
from scipy import stats
from scipy.special import betaln, gammaln

from . import design

OK, DEGENERATE, FAILED, NA = 0, 1, 2, 3
M = design.RUNS_PER_BLOCK


class Data:
    """Analysed E1 indicators of one batch, arranged by arm."""

    def __init__(self, sim):
        e1 = sim['outcome'] == 1                                   # (R, Q, 4, 15)
        order = np.argsort(sim['arm'], axis=2, kind='stable')      # A blocks first
        e1 = np.take_along_axis(e1, order[..., None], axis=2)
        self.reps, self.quartets = e1.shape[:2]
        self.a = e1[:, :, :2].reshape(self.reps, -1, M)            # (R, nb, 15)
        self.b = e1[:, :, 2:].reshape(self.reps, -1, M)
        self.nb = self.a.shape[1]
        self.orient = sim['orient']
        pa, pb = self.a.mean(axis=2), self.b.mean(axis=2)          # block proportions (R, nb)
        self.block_a, self.block_b = pa, pb
        self.quartet_d = pb.reshape(self.reps, -1, 2).mean(2) - pa.reshape(self.reps, -1, 2).mean(2)
        seed = design.SEED_POS
        self.xa = np.stack([self.a[:, :, seed == s].sum((1, 2)) for s in range(3)], 1)  # (R, 3)
        self.xb = np.stack([self.b[:, :, seed == s].sum((1, 2)) for s in range(3)], 1)
        self.n_seed = self.nb * 5                                  # legs per arm per seed


def _closed(est, se, df, thresholds, alpha):
    """Rejections from a point estimate and a t (df finite) or z (df inf) bound."""
    thresholds = np.asarray(thresholds, dtype=float)
    q = np.where(np.isfinite(df), stats.t.ppf(1 - alpha, np.where(np.isfinite(df), df, 1)),
                 stats.norm.ppf(1 - alpha))
    ok = np.isfinite(se) & (se > 0) & np.isfinite(est) & np.isfinite(q)
    upper = est + q * se
    lower = est - q * se
    rej_u = ok[:, None] & (upper[:, None] < thresholds[None, :])
    rej_l = ok[:, None] & (lower[:, None] > thresholds[None, :])
    status = np.where(ok, OK, DEGENERATE).astype(np.int8)
    status[~np.isfinite(est)] = FAILED
    return rej_u, rej_l, status


# --- C0, C1, C6: Miettinen-Nurminen score, stratified by seed ---------------

def fm_restricted(x1, n1, x2, n2, delta):
    """Restricted MLE (p1, p2) under p1 - p2 = delta (Farrington-Manning,
    Miettinen-Nurminen). Group 1 is the candidate. Vectorised; n may be real."""
    p1h, p2h = x1 / n1, x2 / n2
    theta = n2 / n1
    a = 1 + theta
    b = -(1 + theta + p1h + theta * p2h + delta * (theta + 2))
    c = delta ** 2 + delta * (2 * p1h + theta + 1) + p1h + theta * p2h
    d = -p1h * delta * (1 + delta)
    v = b ** 3 / (27 * a ** 3) - b * c / (6 * a ** 2) + d / (2 * a)
    uu = b ** 2 / (9 * a ** 2) - c / (3 * a)
    u = np.sign(v) * np.sqrt(np.maximum(uu, 0))
    with np.errstate(divide='ignore', invalid='ignore'):
        ratio = np.where(u != 0, v / np.where(u != 0, u, 1) ** 3, 0.0)
    w = (np.pi + np.arccos(np.clip(ratio, -1, 1))) / 3
    p1 = 2 * u * np.cos(w) - b / (3 * a)
    p1 = np.clip(p1, max(delta, 0.0), min(1.0, 1 + delta))
    return p1, p1 - delta


def mn_z(xb, nb, xa, na, delta):
    """Stratified MN score statistic for H0: RD = delta, equal stratum weights
    (the design is seed-balanced, so CMH weights are equal). xb, xa: (R, S);
    nb, na: (R, S) or scalars. Returns z (R,), NaN where the variance is 0."""
    nb = np.broadcast_to(nb, xb.shape).astype(float)
    na = np.broadcast_to(na, xa.shape).astype(float)
    p1, p2 = fm_restricted(xb, nb, xa, na, delta)
    corr = (nb + na) / (nb + na - 1)
    var = (p1 * (1 - p1) / nb + p2 * (1 - p2) / na) * corr
    d = xb / nb - xa / na
    s = xb.shape[1]
    num = (d - delta).sum(1) / s
    den = np.sqrt(var.sum(1)) / s
    with np.errstate(divide='ignore', invalid='ignore'):
        return np.where(den > 0, num / den, np.nan)


def _score_family(data, thresholds, alpha, eff_a, eff_b):
    """MN score tests with effective sizes eff_* = n / DE (R, 3)."""
    z_crit = stats.norm.ppf(1 - alpha)
    # Counts scaled with the sizes keep p-hat; the score uses the effective n.
    xa = data.xa * (eff_a / data.n_seed)
    xb = data.xb * (eff_b / data.n_seed)
    k = len(thresholds)
    rej_u = np.zeros((data.reps, k), bool)
    rej_l = np.zeros((data.reps, k), bool)
    status = np.full(data.reps, OK, np.int8)
    for j, d0 in enumerate(thresholds):
        z = mn_z(xb, eff_b, xa, eff_a, d0)
        bad = ~np.isfinite(z)            # zero variance at d0: no rejection at this d0
        rej_u[:, j] = ~bad & (z < -z_crit)
        rej_l[:, j] = ~bad & (z > z_crit)
    return rej_u, rej_l, status


def c0_independent(data, thresholds, alpha=0.05):
    n = np.full((data.reps, 3), float(data.n_seed))
    return _score_family(data, thresholds, alpha, n, n)


def design_effect(block_props, p_hat, m=M):
    """Per-arm variance-ratio design effect: the observed variance of block
    proportions against the binomial variance of a block of m legs, floored
    at 1. Undefined (returned as 1, flagged) when p-hat is 0 or 1."""
    var_b = block_props.var(axis=1, ddof=1)
    binom = p_hat * (1 - p_hat) / m
    with np.errstate(divide='ignore', invalid='ignore'):
        de = np.where(binom > 0, var_b / binom, np.nan)
    return np.maximum(np.nan_to_num(de, nan=1.0), 1.0), binom > 0


def c1_inflated(data, thresholds, alpha=0.05):
    de_a, _ = design_effect(data.block_a, data.block_a.mean(1))
    de_b, _ = design_effect(data.block_b, data.block_b.mean(1))
    n = float(data.n_seed)
    return _score_family(data, thresholds, alpha,
                         np.repeat((n / de_a)[:, None], 3, 1), np.repeat((n / de_b)[:, None], 3, 1))


def c6_conservative(data, thresholds, alpha=0.05):
    """Effective sample size = number of blocks (design effect = 15, full
    within-block correlation). A modelling reference, not a proven bound."""
    n = np.full((data.reps, 3), data.n_seed / M)
    return _score_family(data, thresholds, alpha, n, n)


# --- C2: block-summary Welch -------------------------------------------------

def c2_block_summary(data, thresholds, alpha=0.05):
    a, b = data.block_a, data.block_b
    na, nb = a.shape[1], b.shape[1]
    va, vb = a.var(1, ddof=1) / na, b.var(1, ddof=1) / nb
    est = b.mean(1) - a.mean(1)
    se = np.sqrt(va + vb)
    with np.errstate(divide='ignore', invalid='ignore'):
        df = (va + vb) ** 2 / (va ** 2 / (na - 1) + vb ** 2 / (nb - 1))
    df = np.where(se > 0, df, np.nan)
    return _closed(est, se, df, thresholds, alpha)


# --- C3: quartet-paired t ----------------------------------------------------

def c3_quartet_paired(data, thresholds, alpha=0.05):
    d = data.quartet_d
    q = d.shape[1]
    est = d.mean(1)
    se = d.std(1, ddof=1) / np.sqrt(q)
    return _closed(est, se, np.full(data.reps, q - 1.0), thresholds, alpha)


# --- C4: cluster-robust linear probability model, CR2, Bell-McCaffrey df ----

@lru_cache(maxsize=None)
def cr2_design(nb, seeds=True):
    """Design matrices of the LPM y ~ 1 + armB [+ seed dummies], clusters =
    blocks (A blocks then B blocks). Returns (M (p, n) = (X'X)^-1 X',
    g_a (15,), g_b (15,), df): var-hat = sum_c (g_c . e_c)^2 and the
    Bell-McCaffrey (Imbens-Kolesar) degrees of freedom for the arm contrast."""
    rows = []
    for arm in (0, 1):
        for _ in range(nb):
            for s in design.SEED_POS:
                r = [1.0, float(arm)]
                if seeds:
                    r += [float(s == 1), float(s == 2)]
                rows.append(r)
    X = np.array(rows)
    xtx_inv = np.linalg.inv(X.T @ X)
    Mx = xtx_inv @ X.T
    n = X.shape[0]
    ell = np.zeros(X.shape[1]); ell[1] = 1.0
    H = X @ Mx
    g = []
    adj = []
    for arm in (0, 1):
        sl = slice(arm * nb * M, arm * nb * M + M)              # first block of the arm
        Xc = X[sl]
        Hcc = Xc @ xtx_inv @ Xc.T
        w, V = np.linalg.eigh(np.eye(M) - Hcc)
        Ac = V @ np.diag(1 / np.sqrt(w)) @ V.T
        g.append(Ac @ Xc @ xtx_inv @ ell)
        adj.append(Ac)
    # Bell-McCaffrey df: G's column c = (I - H)[:, c] A_c X_c (X'X)^-1 ell.
    IH = np.eye(n) - H
    cols = []
    for c in range(2 * nb):
        arm = c // nb
        sl = slice(c * M, (c + 1) * M)
        cols.append(IH[:, sl] @ g[arm])
    G = np.array(cols).T
    lam = np.linalg.eigvalsh(G.T @ G)
    df = lam.sum() ** 2 / (lam ** 2).sum()
    return Mx, g[0], g[1], float(df)


def c4_cluster_gee(data, thresholds, alpha=0.05, seeds=True):
    """GEE with identity link, Gaussian working variance and independence
    working correlation (a linear probability model), arm and seed terms,
    CR2 variance and Bell-McCaffrey df."""
    nb = data.nb
    Mx, g_a, g_b, df = cr2_design(nb, seeds)
    y = np.concatenate([data.a, data.b], axis=1).reshape(data.reps, -1).astype(float)
    beta = y @ Mx.T                                                  # (R, p)
    p = Mx.shape[0]
    rows = []
    for arm in (0, 1):
        r = np.zeros((M, p)); r[:, 0] = 1; r[:, 1] = arm
        if seeds:
            r[:, 2] = design.SEED_POS == 1; r[:, 3] = design.SEED_POS == 2
        rows.append(r)
    fit_a = beta @ rows[0].T                                         # (R, 15)
    fit_b = beta @ rows[1].T
    ea = data.a - fit_a[:, None, :]
    eb = data.b - fit_b[:, None, :]
    var = ((ea @ g_a) ** 2).sum(1) + ((eb @ g_b) ** 2).sum(1)
    est = beta[:, 1]
    return _closed(est, np.sqrt(var), np.full(data.reps, df), thresholds, alpha)


# --- C5: randomisation inference over the balanced orientation set ----------

C5_MAX_PATTERNS = 924                                                # Q <= 12


def c5_attainable(quartets):
    """Smallest attainable one-sided p over the balanced assignment set."""
    from math import comb
    return 1.0 / comb(quartets, quartets // 2)


def c5_randomisation(data, thresholds, alpha=0.05):
    """Sharp-null test inversion under a constant additive shift on the
    block-proportion scale: under H0 RD = d0, relabelling a quartet maps its
    centred difference z_q = d_q - d0 to -z_q. Reference set: every balanced
    orientation pattern. p-value includes the observed pattern."""
    q = data.quartets
    k = len(thresholds)
    from math import comb
    if comb(q, q // 2) > C5_MAX_PATTERNS:
        z = np.zeros((data.reps, k), bool)
        return z, z.copy(), np.full(data.reps, NA, np.int8)
    pats = design.balanced_patterns(q)                               # (P, Q)
    signs = np.where(pats[None, :, :] == data.orient[:, None, :], 1.0, -1.0)  # (R, P, Q)
    rej_u = np.zeros((data.reps, k), bool)
    rej_l = np.zeros((data.reps, k), bool)
    eps = 1e-12
    for j, d0 in enumerate(thresholds):
        z = data.quartet_d - d0                                      # (R, Q)
        t_obs = z.mean(1)
        t_ref = (signs * z[:, None, :]).mean(2)                      # (R, P)
        p_low = (t_ref <= t_obs[:, None] + eps).mean(1)             # evidence RD < d0
        p_high = (t_ref >= t_obs[:, None] - eps).mean(1)            # evidence RD > d0
        rej_u[:, j] = p_low <= alpha
        rej_l[:, j] = p_high <= alpha
    return rej_u, rej_l, np.full(data.reps, OK, np.int8)


# --- C7: beta-binomial profile likelihood -----------------------------------

C7_STEP = 0.0025
C7_MU = np.arange(C7_STEP / 2, 0.6, C7_STEP)                         # block-mean grid
C7_RHO = np.concatenate([[0.0], np.geomspace(1e-3, 0.6, 30)])       # intra-block correlation


@lru_cache(maxsize=1)
def _bb_table():
    """log P(k | mu, rho) for k = 0..15: (len(mu), len(rho), 16)."""
    k = np.arange(M + 1)
    mu = C7_MU[:, None, None]
    rho = C7_RHO[None, :, None]
    logc = gammaln(M + 1) - gammaln(k + 1) - gammaln(M - k + 1)
    with np.errstate(divide='ignore', invalid='ignore'):
        a = mu * (1 - rho) / rho
        b = (1 - mu) * (1 - rho) / rho
        bb = logc + betaln(k + a, M - k + b) - betaln(a, b)
    binom = logc + k * np.log(mu) + (M - k) * np.log1p(-mu)
    return np.where(rho == 0, binom, bb)


def c7_beta_binomial(data, thresholds, alpha=0.05, chunk=500):
    """Shared overdispersion rho, block means mu_A, mu_B (seed-balanced
    blocks, seeds not modelled). Signed-root profile likelihood ratio on a
    grid with step 0.0025 for mu; thresholds must be multiples of the step."""
    table = _bb_table().reshape(len(C7_MU) * len(C7_RHO), M + 1)    # (I*J, 16)
    steps = np.rint(np.asarray(thresholds) / C7_STEP).astype(int)
    if not np.allclose(steps * C7_STEP, thresholds):
        raise ValueError('C7 thresholds must be multiples of the grid step')
    z_crit = stats.norm.ppf(1 - alpha)
    ka = data.a.sum(2)                                               # (R, nb) block counts
    kb = data.b.sum(2)
    ha = np.stack([(ka == k).sum(1) for k in range(M + 1)], 1).astype(float)
    hb = np.stack([(kb == k).sum(1) for k in range(M + 1)], 1).astype(float)
    k = len(thresholds)
    rej_u = np.zeros((data.reps, k), bool)
    rej_l = np.zeros((data.reps, k), bool)
    status = np.full(data.reps, OK, np.int8)
    I, J = len(C7_MU), len(C7_RHO)
    for s in range(0, data.reps, chunk):
        e = slice(s, s + chunk)
        la = (ha[e] @ table.T).reshape(-1, I, J)
        lb = (hb[e] @ table.T).reshape(-1, I, J)
        best = la.max(1) + lb.max(1)                                 # (r, J)
        lmax = best.max(1)
        j_star = best.argmax(1)
        r_idx = np.arange(la.shape[0])
        ia = la[r_idx, :, j_star].argmax(1)
        ib = lb[r_idx, :, j_star].argmax(1)
        d_hat = (ib - ia) * C7_STEP
        edge = (ia == I - 1) | (ib == I - 1)
        status[e][edge] = FAILED
        for j, st in enumerate(steps):
            if st >= 0:
                prof = (la[:, :I - st, :] + lb[:, st:, :]).max((1, 2)) if st < I else np.full(len(lmax), -np.inf)
            else:
                prof = (la[:, -st:, :] + lb[:, :I + st, :]).max((1, 2))
            lr = np.maximum(2 * (lmax - prof), 0)
            root = np.sign(d_hat - thresholds[j]) * np.sqrt(lr)
            good = ~edge & np.isfinite(root)
            rej_u[e, j] = good & (root < -z_crit)
            rej_l[e, j] = good & (root > z_crit)
    return rej_u, rej_l, status


def _uniform_degeneracy(method):
    """The common sparse-event rule, applied identically to every method: an
    experiment with no E1 event at all, or with every leg an E1 event, gives
    no comparative conclusion. It is DEGENERATE and rejects nothing. No
    method substitutes another interval in that case."""
    def wrapped(data, thresholds, alpha=0.05):
        rej_u, rej_l, status = method(data, thresholds, alpha=alpha)
        total = data.a.sum((1, 2)) + data.b.sum((1, 2))
        legs = data.a[0].size + data.b[0].size
        degenerate = (total == 0) | (total == legs)
        rej_u = rej_u & ~degenerate[:, None]
        rej_l = rej_l & ~degenerate[:, None]
        status = np.where(degenerate & (status != NA), DEGENERATE, status).astype(np.int8)
        bad = status != OK
        return rej_u & ~bad[:, None], rej_l & ~bad[:, None], status
    wrapped.__name__ = method.__name__
    wrapped.__doc__ = method.__doc__
    return wrapped


METHODS = {name: _uniform_degeneracy(f) for name, f in {
    'C0': c0_independent,
    'C1': c1_inflated,
    'C2': c2_block_summary,
    'C3': c3_quartet_paired,
    'C4': c4_cluster_gee,
    'C5': c5_randomisation,
    'C6': c6_conservative,
    'C7': c7_beta_binomial,
}.items()}
