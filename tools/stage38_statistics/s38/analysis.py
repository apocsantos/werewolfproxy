"""Operating characteristics from runner output (handoff sections 12-13).

    python -m s38.analysis RESULT_DIR OUT_PREFIX

Writes OUT_PREFIX.csv (one row per scenario and method) and prints nothing
else. Every rate is over valid experiments (an experiment beyond the
replacement cap is invalid, never a statistical result) and carries one-sided
95% Clopper-Pearson Monte Carlo bounds.
"""
import csv
import glob
import json
import os
import sys

import numpy as np
from scipy import stats

MARGINS = (0.05, 0.075, 0.10)
STATUS = ('ok', 'degenerate', 'failed', 'na')


def cp_upper(k, n, conf=0.95):
    return 1.0 if k >= n else float(stats.beta.ppf(conf, k + 1, n - k))


def cp_lower(k, n, conf=0.95):
    return 0.0 if k <= 0 else float(stats.beta.ppf(1 - conf, k, n - k + 1))


def load(result_dir):
    out = []
    for path in sorted(glob.glob(os.path.join(result_dir, '[0-9]*.json'))):
        with open(path) as f:
            out.append(json.load(f))
    return out


def rows(results):
    for r in results:
        if 'skipped' in r:
            yield {'index': r['index'], 'group': r['group'], **r['cell'], 'skipped': r['skipped']}
            continue
        th = r['thresholds']
        n = r['valid']
        true = round(r['true_rd'], 6)
        it = th.index(true)
        for m, a in r['methods'].items():
            row = {'index': r['index'], 'group': r['group'], **r['cell'], 'method': m,
                   'true_rd': true, 'reps': r['reps'], 'valid': n,
                   'replacements': r['replacements'], 'zero_events': r['zero_events'],
                   'one_sided_zero': r['one_sided_zero'], 'seconds': round(a['seconds'], 3)}
            for s, c in zip(STATUS, a['status']):
                row[f'status_{s}'] = c
            cov_u = n - a['rej_u'][it]
            cov_l = n - a['rej_l'][it]
            row.update(cov_u=cov_u / n if n else None, cov_u_cp_upper=cp_upper(cov_u, n) if n else None,
                       cov_l=cov_l / n if n else None, cov_l_cp_upper=cp_upper(cov_l, n) if n else None)
            for d in MARGINS:
                k = a['rej_u'][th.index(d)]
                key = f'{d * 1000:g}'.replace('.', '_')
                row[f'ni_{key}'] = k / n if n else None            # P(U < delta)
                row[f'ni_{key}_cp_upper'] = cp_upper(k, n) if n else None
                row[f'ni_{key}_cp_lower'] = cp_lower(k, n) if n else None
                ki = a['rej_l'][th.index(d)]
                row[f'inf_{key}'] = ki / n if n else None          # P(L > delta)
            k0 = a['rej_l'][th.index(0.0)]
            row['worse'] = k0 / n if n else None                   # P(L > 0)
            row['worse_cp_upper'] = cp_upper(k0, n) if n else None
            yield row
    return


def main(argv=None):
    argv = argv or sys.argv[1:]
    result_dir, prefix = argv
    data = list(rows(load(result_dir)))
    keys = []
    for d in data:
        for k in d:
            if k not in keys:
                keys.append(k)
    with open(prefix + '.csv', 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=keys)
        w.writeheader()
        w.writerows(data)


if __name__ == '__main__':
    main()
