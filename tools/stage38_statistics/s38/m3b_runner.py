"""Resumable, deterministic M3b runner (same contract as s38.runner).

    python -m s38.m3b_runner CONFIG.json OUT_DIR [--workers N]

Per cell and method: rejections for E1 and E2 at their thresholds, the
joint E1-then-E2 hierarchy outcome, statuses, and the sum of the plain
mean-difference estimate (for bias). Each method gets its own stream:
SeedSequence(root, spawn_key=(cell, chunk, 1000 + method index)).
"""
import argparse
import hashlib
import json
import os
import sys
import time
from multiprocessing import Pool

import numpy as np

from . import m3b, m3b_scenarios

E1_MARGINS = (0.05, 0.075, 0.10)
E2_MARGIN = 0.10


def _hash(obj):
    return hashlib.sha256(json.dumps(obj, sort_keys=True).encode()).hexdigest()[:16]


def run_cell(args):
    index, group, cell, cfg, out_dir = args
    path = os.path.join(out_dir, f'{index:06d}.json')
    h = _hash({'cfg': cfg, 'cell': cell, 'group': group})
    if os.path.exists(path):
        with open(path) as f:
            if json.load(f).get('hash') == h:
                return path
    res = {'index': index, 'group': group, 'cell': cell, 'hash': h}
    try:
        sc = m3b.Scenario(**cell)
    except ValueError as error:
        res['skipped'] = str(error)
        _write(path, res)
        return path
    th1 = sorted({round(sc.true_rd, 6), 0.0, *E1_MARGINS})
    th2 = sorted({round(sc.true_rd2, 6), 0.0, E2_MARGIN})
    methods = cfg['methods']
    zero = lambda k: np.zeros(k, np.int64)
    agg = {m: {'e1': {'rej_u': zero(len(th1)), 'rej_l': zero(len(th1)), 'status': zero(4)},
               'e2': {'rej_u': zero(len(th2)), 'rej_l': zero(len(th2)), 'status': zero(4)},
               'overall_ni': 0, 'overall_inf': 0, 'seconds': 0.0} for m in methods}
    valid = run_blocks = 0
    rd_hat = rd2_hat = 0.0
    reps, chunk = cfg['reps'], cfg.get('chunk', 1000)
    for c, start in enumerate(range(0, reps, chunk)):
        n = min(chunk, reps - start)
        rng = np.random.default_rng(np.random.SeedSequence(cfg['root_seed'], spawn_key=(index, c)))
        sim = m3b.simulate(rng, sc, n)
        keep = sim['valid']
        run_blocks += int(sim['run'].sum())
        valid += int(keep.sum())
        if not keep.any():
            continue
        sub = {k: v[keep] for k, v in sim.items()}
        d1, d2 = m3b.data(sub, 1), m3b.data(sub, 2)
        rd_hat += float((d1.b.mean((1, 2)) - d1.a.mean((1, 2))).sum())
        rd2_hat += float((d2.b.mean((1, 2)) - d2.a.mean((1, 2))).sum())
        for k, m in enumerate(methods):
            mrng = np.random.default_rng(np.random.SeedSequence(cfg['root_seed'], spawn_key=(index, c, 1000 + k)))
            t0 = time.perf_counter()
            ru1, rl1, st1 = m3b.call(m, d1, th1, rng=mrng)
            ru2, rl2, st2 = m3b.call(m, d2, th2, rng=mrng)
            agg[m]['seconds'] += time.perf_counter() - t0
            for key, ru, rl, st in (('e1', ru1, rl1, st1), ('e2', ru2, rl2, st2)):
                agg[m][key]['rej_u'] += ru.sum(0)
                agg[m][key]['rej_l'] += rl.sum(0)
                agg[m][key]['status'] += np.bincount(st, minlength=4)
            e1_ni, e1_inf = ru1[:, th1.index(0.075)], rl1[:, th1.index(0.075)]
            e2_ni, e2_inf = ru2[:, th2.index(E2_MARGIN)], rl2[:, th2.index(E2_MARGIN)]
            agg[m]['overall_ni'] += int((e1_ni & e2_ni).sum())
            agg[m]['overall_inf'] += int((e1_inf | (e1_ni & e2_inf)).sum())
    res.update({'true_rd': sc.true_rd, 'true_rd2': sc.true_rd2, 'th1': th1, 'th2': th2,
                'reps': reps, 'valid': valid, 'blocks_run': run_blocks,
                'rd_hat_sum': rd_hat, 'rd2_hat_sum': rd2_hat,
                'methods': {m: {'e1': {k: v.tolist() for k, v in a['e1'].items()},
                                'e2': {k: v.tolist() for k, v in a['e2'].items()},
                                'overall_ni': a['overall_ni'], 'overall_inf': a['overall_inf'],
                                'seconds': a['seconds']} for m, a in agg.items()}})
    _write(path, res)
    return path


def _write(path, obj):
    tmp = path + '.tmp'
    with open(tmp, 'w') as f:
        json.dump(obj, f)
    os.replace(tmp, path)


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument('config')
    ap.add_argument('out')
    ap.add_argument('--workers', type=int, default=os.cpu_count())
    args = ap.parse_args(argv)
    with open(args.config) as f:
        cfg = json.load(f)
    os.makedirs(args.out, exist_ok=True)
    cells = m3b_scenarios.LISTS[cfg['scenarios']](**cfg.get('scenario_args', {}))
    jobs = [(i, g, c, cfg, args.out) for i, (g, c) in enumerate(cells)]
    with open(os.path.join(args.out, 'manifest.json'), 'w') as f:
        json.dump({'config': cfg, 'cells': len(jobs), 'python': sys.version,
                   'numpy': np.__version__}, f, indent=1)
    t0 = time.time()
    with Pool(args.workers) as pool:
        for k, _ in enumerate(pool.imap_unordered(run_cell, jobs, chunksize=1), 1):
            if k % 50 == 0 or k == len(jobs):
                print(f'{k}/{len(jobs)} cells, {time.time() - t0:.0f} s', flush=True)


if __name__ == '__main__':
    main()
