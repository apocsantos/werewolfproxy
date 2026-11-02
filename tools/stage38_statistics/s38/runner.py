"""Resumable, deterministic simulation runner.

    python -m s38.runner CONFIG.json OUT_DIR [--workers N]

Each scenario's result is one JSON file in OUT_DIR, written atomically; an
existing file with the same configuration hash is skipped, so a run resumes
where it stopped. Random streams: SeedSequence(root, spawn_key=(scenario,
chunk)), so results do not depend on worker count or order. A replicate is
never regenerated: numerical failures are counted, not retried.
"""
import argparse
import hashlib
import json
import os
import sys
import time
from multiprocessing import Pool

import numpy as np

from . import estimators, generator, scenarios

MARGINS = (0.05, 0.075, 0.10)


def config_hash(cfg):
    return hashlib.sha256(json.dumps(cfg, sort_keys=True).encode()).hexdigest()[:16]


def thresholds_for(true_rd):
    vals = {round(true_rd, 6), 0.0, *MARGINS}
    return sorted(vals)


def run_scenario(args):
    index, group, cell, cfg, out_dir = args
    path = os.path.join(out_dir, f'{index:06d}.json')
    h = config_hash({'cfg': cfg, 'cell': cell, 'group': group})
    if os.path.exists(path):
        with open(path) as f:
            if json.load(f).get('hash') == h:
                return path
    result = {'index': index, 'group': group, 'cell': cell, 'hash': h}
    try:
        sc = generator.Scenario(**cell)
    except ValueError as error:
        result['skipped'] = str(error)
        _write(path, result)
        return path
    th = thresholds_for(sc.true_rd)
    methods = cfg.get('methods', list(estimators.METHODS))
    agg = {m: {'rej_u': np.zeros(len(th), np.int64), 'rej_l': np.zeros(len(th), np.int64),
               'status': np.zeros(4, np.int64), 'seconds': 0.0} for m in methods}
    interim = cfg.get('interim', [])
    inter = {m: {str(a): {'stop': 0, 'stop_and_ni': 0, 'ni_not_stopped': 0} for a in interim}
             for m in methods if interim}
    valid_total = replaced_total = zero_events = one_sided_zero = 0
    reps, chunk = cfg['reps'], cfg.get('chunk', 2000)
    for c, start in enumerate(range(0, reps, chunk)):
        n = min(chunk, reps - start)
        rng = np.random.default_rng(np.random.SeedSequence(cfg['root_seed'], spawn_key=(index, c)))
        sim = generator.simulate(rng, sc, n)
        keep = sim['valid']
        valid_total += int(keep.sum())
        replaced_total += int(sim['replaced'][keep].sum())
        sub = {k: v[keep] for k, v in sim.items()}
        if not keep.any():
            continue
        data = estimators.Data(sub)
        ea, eb = data.a.sum((1, 2)), data.b.sum((1, 2))
        zero_events += int(((ea == 0) & (eb == 0)).sum())
        one_sided_zero += int(((ea == 0) ^ (eb == 0)).sum())
        half = None
        if interim:
            q2 = sc.quartets // 2
            half = estimators.Data({'outcome': sub['outcome'][:, :q2], 'arm': sub['arm'][:, :q2],
                                    'orient': sub['orient'][:, :q2]})
        for m in methods:
            t0 = time.perf_counter()
            ru, rl, st = estimators.METHODS[m](data, th)
            agg[m]['seconds'] += time.perf_counter() - t0
            agg[m]['rej_u'] += ru.sum(0)
            agg[m]['rej_l'] += rl.sum(0)
            agg[m]['status'] += np.bincount(st, minlength=4)
            if interim and q2 >= 2:
                ni = ru[:, th.index(0.075)]
                for a in interim:
                    _, rl_i, _ = estimators.METHODS[m](half, [0.0], alpha=a)
                    stop = rl_i[:, 0]
                    d = inter[m][str(a)]
                    d['stop'] += int(stop.sum())
                    d['stop_and_ni'] += int((stop & ni).sum())
                    d['ni_not_stopped'] += int((~stop & ni).sum())
    result.update({
        'true_rd': sc.true_rd, 'thresholds': th, 'reps': reps, 'valid': valid_total,
        'replacements': replaced_total, 'zero_events': zero_events,
        'one_sided_zero': one_sided_zero,
        'methods': {m: {k: (v.tolist() if isinstance(v, np.ndarray) else v) for k, v in a.items()}
                    for m, a in agg.items()},
    })
    if interim:
        result['interim'] = inter
    _write(path, result)
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
    cells = scenarios.LISTS[cfg['scenarios']](**cfg.get('scenario_args', {}))
    jobs = [(i, g, c, cfg, args.out) for i, (g, c) in enumerate(cells)]
    with open(os.path.join(args.out, 'manifest.json'), 'w') as f:
        json.dump({'config': cfg, 'scenarios': len(jobs), 'python': sys.version,
                   'numpy': np.__version__}, f, indent=1)
    t0 = time.time()
    with Pool(args.workers) as pool:
        for k, _ in enumerate(pool.imap_unordered(run_scenario, jobs, chunksize=4), 1):
            if k % 200 == 0 or k == len(jobs):
                print(f'{k}/{len(jobs)} scenarios, {time.time() - t0:.0f} s', flush=True)


if __name__ == '__main__':
    main()
