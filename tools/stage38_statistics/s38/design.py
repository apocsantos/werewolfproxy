"""The experiment's structure: blocks, seeds, quartets, orientation and
whole-quartet replacement (spec sections 3 and 9; owner decisions V3, V5).

A block is one sandbox running 15 TCP-hop legs in the harness's seed order
2010, 2020, 2030, repeated five times. A quartet is four blocks in the order
ABBA or BAAB. Exactly half the quartets of an experiment are ABBA (balanced,
V3). Arm codes: 0 = A (control), 1 = B (candidate).
"""
import numpy as np

SEEDS = (2010, 2020, 2030)
RUNS_PER_BLOCK = 15
SEED_POS = np.tile(np.arange(3), 5)          # seed index of each run in a block
ABBA, BAAB = 0, 1
ARMS = {ABBA: np.array([0, 1, 1, 0]), BAAB: np.array([1, 0, 0, 1])}
MAX_REPLACEMENTS = 2                         # V5


def balanced_orientations(rng, reps, quartets):
    """(reps, quartets) orientations, exactly half ABBA in each row."""
    if quartets % 2:
        raise ValueError('a balanced design needs an even number of quartets')
    base = np.repeat([ABBA, BAAB], quartets // 2)
    return rng.permuted(np.tile(base, (reps, 1)), axis=1)


def arms_of(orientation):
    """(..., Q) orientations to (..., Q, 4) arm codes."""
    return np.where(orientation[..., None] == ABBA, ARMS[ABBA], ARMS[BAAB])


def schedule(rng, reps, quartets, p_invalid, cap=MAX_REPLACEMENTS):
    """Whole-quartet replacement with an objective, outcome-independent
    invalidation process: each block is invalid with probability p_invalid
    (a suspend, a binary mismatch, a harness failure, missing records),
    drawn before and independently of any outcome.

    Slots 0..Q-1 are the planned quartets; slots Q..Q+cap-1 are the
    replacement quartets, run afterwards in that order. An invalid quartet is
    replaced by the next replacement slot, which takes its orientation so that
    the analysed quartets stay balanced. A replacement that is itself invalid
    uses up the cap too.

    Returns (orientation (reps, Q+cap), analysed (reps, Q) slot indices,
    valid (reps,) experiment validity, replacements_used (reps,)).
    """
    slots = quartets + cap
    orientation = np.zeros((reps, slots), dtype=np.int8)
    orientation[:, :quartets] = balanced_orientations(rng, reps, quartets)
    bad = rng.random((reps, slots, 4)) < p_invalid
    bad_quartet = bad.any(axis=2)
    analysed = np.tile(np.arange(quartets), (reps, 1))
    valid = np.ones(reps, dtype=bool)
    used = np.zeros(reps, dtype=np.int8)
    for r in np.flatnonzero(bad_quartet[:, :quartets].any(axis=1)):
        nxt = quartets
        for q in np.flatnonzero(bad_quartet[r, :quartets]):
            while True:
                if nxt >= slots:
                    valid[r] = False
                    break
                orientation[r, nxt] = orientation[r, q]
                nxt += 1
                if not bad_quartet[r, nxt - 1]:
                    analysed[r, q] = nxt - 1
                    break
            if not valid[r]:
                break
        used[r] = nxt - quartets
    return orientation, analysed, valid, used


def balanced_patterns(quartets):
    """Every balanced orientation pattern: (C(Q, Q/2), Q)."""
    from itertools import combinations
    rows = []
    for abba in combinations(range(quartets), quartets // 2):
        row = np.ones(quartets, dtype=np.int8)
        row[list(abba)] = ABBA
        rows.append(row)
    return np.array(rows, dtype=np.int8)
