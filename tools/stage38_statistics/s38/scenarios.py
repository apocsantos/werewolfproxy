"""Scenario lists (spec section 6.2; the M3 handoff's section 8).

Probabilities and RD are fractions. Infeasible cells (a candidate seed rate
outside (0, 1)) are recorded as skipped, never silently dropped.
"""
import itertools

P = (0.02, 0.04, 0.06)
RD = (-0.02, -0.01, 0.0, 0.025, 0.05, 0.075, 0.10, 0.15)
SIGMA = (0.0, 0.5, 1.0, 1.5)
SIGMA_STRONG = (2.0, 2.5)
QUARTETS = (2, 4, 6, 8)                     # 60, 120, 180, 240 legs per arm
DRIFT = (('none', 0.0), ('linear', 0.5), ('linear', -0.5), ('step', 0.5))


def _cell(**kw):
    base = dict(p_a=0.04, rd=0.0, sigma=0.0, quartets=4, asym=0.0, phi=0.0,
                seeds='equal', drift='none', tau=0.0, p_invalid=0.0)
    base.update(kw)
    return base


def screening():
    cells = []
    # A. core factorial: rate x RD x dependence x size
    for p, rd, s, q in itertools.product(P, RD, SIGMA, QUARTETS):
        cells.append(('A', _cell(p_a=p, rd=rd, sigma=s, quartets=q)))
    # B. nuisance factorial at the decision-relevant RDs
    for p, rd, s, q, asym, phi, seeds, (dr, tau) in itertools.product(
            P, (0.0, 0.05, 0.075, 0.10), SIGMA + SIGMA_STRONG, (4, 8), (0.0, 0.5), (0.0, 0.5),
            ('equal', 'seed2030x3'), DRIFT):
        cells.append(('B', _cell(p_a=p, rd=rd, sigma=s, quartets=q, asym=asym, phi=phi,
                                 seeds=seeds, drift=dr, tau=tau)))
    # C. objective invalidation with whole-quartet replacement
    for p, rd, s, q, pinv, (dr, tau) in itertools.product(
            (0.02, 0.06), (0.0, 0.075), (0.0, 1.5), (4, 8), (0.05, 0.10),
            (('none', 0.0), ('linear', 0.5))):
        cells.append(('C', _cell(p_a=p, rd=rd, sigma=s, quartets=q, p_invalid=pinv,
                                 drift=dr, tau=tau)))
    # D. sparse events: rates below the historical range
    for p, rd, s, q in itertools.product((0.005, 0.01), (0.0, 0.075), (0.0, 1.5), (4, 8)):
        cells.append(('D', _cell(p_a=p, rd=rd, sigma=s, quartets=q)))
    return cells


def confirmation(quartets=(4, 8)):
    """The required validation region at every margin, for the finalists:
    the null at the margin (RD = delta) and beyond it, and RD = 0 for power,
    over every dependence, carry-over, seed and drift combination."""
    cells = []
    for p, rd, s, q, asym, phi, seeds, (dr, tau) in itertools.product(
            P, (0.0, 0.05, 0.075, 0.10, 0.15), SIGMA + SIGMA_STRONG, quartets, (0.0, 0.5),
            (0.0, 0.5), ('equal', 'seed2030x3'), DRIFT):
        cells.append(('F', _cell(p_a=p, rd=rd, sigma=s, quartets=q, asym=asym, phi=phi,
                                 seeds=seeds, drift=dr, tau=tau)))
    for p, rd, s, q, pinv in itertools.product(P, (0.0, 0.075), SIGMA, quartets, (0.05, 0.10)):
        cells.append(('G', _cell(p_a=p, rd=rd, sigma=s, quartets=q, p_invalid=pinv,
                                 drift='linear', tau=0.5)))
    return cells


def power(quartets=(2, 4, 6, 8, 12, 16, 24, 32)):
    """Sample size: RD = 0 and the margins' nulls across sizes and dependence."""
    cells = []
    for p, rd, s, q in itertools.product(P, (0.0, 0.05, 0.075, 0.10), SIGMA, quartets):
        cells.append(('S', _cell(p_a=p, rd=rd, sigma=s, quartets=q)))
    return cells


def interim(quartets=(48,)):
    """Harm-only interim look at half the quartets (simulated, not approved)."""
    cells = []
    for p, rd, s, q, (dr, tau) in itertools.product(P, (0.0, 0.075, 0.10, 0.15), (0.0, 1.0, 1.5), quartets,
                                                     (('none', 0.0), ('linear', 0.5))):
        cells.append(('I', _cell(p_a=p, rd=rd, sigma=s, quartets=q, drift=dr, tau=tau)))
    return cells


LISTS = {'screening': screening, 'confirmation': confirmation, 'power': power, 'interim': interim}
