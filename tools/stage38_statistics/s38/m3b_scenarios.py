"""M3b cell lists (frozen plan sections 3-6). Fractions throughout."""
import itertools

P = (0.02, 0.04, 0.06)
NULL_AND_POWER = (0.0, 0.075, 0.10, 0.15)          # RD = 0, delta, delta + 2.5, delta + 7.5
SIZES = (4, 8, 16, 24)
VARIANTS = (('G2', {'mix_pi': 0.05}), ('G2', {'mix_pi': 0.15}), ('G3ar', {'env_rho': 0.5}),
            ('G3ar', {'env_rho': 0.9}), ('G3step', {}))
DRIFT = (('none', 0.0), ('linear', 0.5), ('linear', -0.5), ('step', 0.5))


def _cell(**kw):
    base = dict(family='G1', p_a=0.04, rd=0.0, e2_a=0.12, rd2=0.0, lam2=1.0, sigma=0.0,
                quartets=8)
    base.update(kw)
    return base


def screening():
    cells = []
    for p, rd, s, q in itertools.product(P, NULL_AND_POWER, (0.0, 0.5, 1.0, 1.5, 2.5), SIZES):
        cells.append(('G1', _cell(p_a=p, rd=rd, sigma=s, quartets=q)))
    for (fam, kw), p, rd, s, q in itertools.product(VARIANTS, P, NULL_AND_POWER, (0.0, 1.0), SIZES):
        cells.append((fam, _cell(family=fam, p_a=p, rd=rd, sigma=s, quartets=q, **kw)))
    for rd, asym, phi, seeds, (dr, tau), q in itertools.product(
            (0.0, 0.075), (0.0, 0.5), (0.0, 0.5), ('equal', 'seed2030x3'), DRIFT, (8, 24)):
        cells.append(('G1n', _cell(p_a=0.04, rd=rd, sigma=1.5, asym=asym, phi=phi, seeds=seeds,
                                   drift=dr, tau=tau, quartets=q)))
    return cells


def power(sizes=(16, 24, 32, 48, 64)):
    cells = []
    for p, rd, s, q in itertools.product(P, (0.0, 0.075), (0.0, 0.5, 1.0, 1.5), sizes):
        cells.append(('S', _cell(p_a=p, rd=rd, sigma=s, quartets=q)))
    return cells


def confirmation(sizes=(32, 48)):
    """Survivors over every family at the sizes where they have power."""
    cells = []
    for p, rd, s, q in itertools.product(P, NULL_AND_POWER, (0.0, 0.5, 1.0, 1.5, 2.5), sizes):
        cells.append(('G1', _cell(p_a=p, rd=rd, sigma=s, quartets=q)))
    for (fam, kw), p, rd, s, q in itertools.product(VARIANTS, P, NULL_AND_POWER, (0.0, 1.0, 1.5), sizes):
        cells.append((fam, _cell(family=fam, p_a=p, rd=rd, sigma=s, quartets=q, **kw)))
    for p, rd, asym, phi, seeds, (dr, tau), q in itertools.product(
            P, (0.0, 0.075), (0.0, 0.5), (0.0, 0.5), ('equal', 'seed2030x3'), DRIFT, sizes):
        cells.append(('G1n', _cell(p_a=p, rd=rd, sigma=1.5, asym=asym, phi=phi, seeds=seeds,
                                   drift=dr, tau=tau, quartets=q)))
    return cells


def hierarchy(sizes=(48,)):
    """E1/E2 combinations (handoff section 8): (E1 RD, E2 RD)."""
    combos = [(0.0, 0.0), (0.0, 0.10), (0.0, 0.15), (0.075, -0.05), (0.075, 0.10), (0.10, 0.15),
              (0.075, 0.0)]
    cells = []
    for (rd, rd2), p, s, q, lam2 in itertools.product(combos, (0.02, 0.04), (0.0, 1.5), sizes,
                                                      (0.0, 1.0)):
        cells.append(('H', _cell(p_a=p, rd=rd, rd2=rd2, sigma=s, quartets=q, lam2=lam2)))
    for rd, q in itertools.product((0.0, 0.075), sizes):          # E2 has no events at all
        cells.append(('Hz2', _cell(p_a=0.04, rd=rd, e2_a=0.0, rd2=0.0, sigma=1.0, quartets=q)))
    for rd, rd2, q in itertools.product((0.0, 0.075), (0.0, 0.10), sizes):   # E1 very sparse
        cells.append(('Hz1', _cell(p_a=0.005, rd=rd, rd2=rd2, sigma=1.0, quartets=q)))
    return cells


def replacement(sizes=(8, 24, 48)):
    models = ([('indep', b) for b in (0.001, 0.005, 0.01, 0.05)] +
              [('quartet', b) for b in (0.01, 0.05)] +
              [('host', b) for b in (0.01, 0.05)] +
              [('late', b) for b in (0.01, 0.05)])
    policies = (('cap', 2), ('cap', 6), ('immediate', 2), ('restart', 3))
    cells = []
    for (model, b), pol, q, rd in itertools.product(models, policies, sizes, (0.0, 0.075)):
        fam = dict(family='G3ar', env_rho=0.9) if model == 'host' else dict(family='G1')
        cells.append(('RP', _cell(p_a=0.04, rd=rd, sigma=1.0, quartets=q, invalid=[model, b],
                                  policy=list(pol), **fam)))
    return cells


def r1_null(sizes=(48,)):
    """R1 error-control confirmation: null cells only, at the size where R1
    has power. Subset fixed for cost before any confirmation result."""
    cells = []
    for p, rd, s, q in itertools.product(P, NULL_AND_POWER[1:], (0.0, 1.5, 2.5), sizes):
        cells.append(('G1', _cell(p_a=p, rd=rd, sigma=s, quartets=q)))
    for (fam, kw), p, rd, s, q in itertools.product(VARIANTS, P, NULL_AND_POWER[1:], (0.0, 1.5), sizes):
        cells.append((fam, _cell(family=fam, p_a=p, rd=rd, sigma=s, quartets=q, **kw)))
    for asym, phi, seeds, (dr, tau), q in itertools.product(
            (0.0, 0.5), (0.0, 0.5), ('equal', 'seed2030x3'), DRIFT, sizes):
        cells.append(('G1n', _cell(p_a=0.04, rd=0.075, sigma=1.5, asym=asym, phi=phi, seeds=seeds,
                                   drift=dr, tau=tau, quartets=q)))
    return cells


LISTS = {'r1_null': r1_null, 'screening': screening, 'power': power, 'confirmation': confirmation,
         'hierarchy': hierarchy, 'replacement': replacement}
