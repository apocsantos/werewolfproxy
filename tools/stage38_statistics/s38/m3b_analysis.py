"""M3b operating characteristics under the frozen plan.

    python -m s38.m3b_analysis RESULT_DIR OUT_CSV

One row per (cell, method, endpoint). Coverage is classified three ways
(plan section 2.2): adequate when the Monte Carlo lower bound is at least
0.95, under when the upper bound is below 0.95, otherwise inconclusive.
"""
import csv
import glob
import json
import os
import sys

from .analysis import cp_lower, cp_upper


def coverage_class(covered, n):
    if n == 0:
        return 'no-valid-experiments'
    if cp_lower(covered, n) >= 0.95:
        return 'adequate'
    if cp_upper(covered, n) < 0.95:
        return 'under'
    return 'inconclusive'


def rows(result_dir):
    for path in sorted(glob.glob(os.path.join(result_dir, '[0-9]*.json'))):
        with open(path) as f:
            r = json.load(f)
        base = {'index': r['index'], 'group': r['group'], **r['cell']}
        if 'skipped' in r:
            yield {**base, 'skipped': r['skipped']}
            continue
        n = r['valid']
        for m, a in r['methods'].items():
            for ep, th_key, true_key, margins in (('E1', 'th1', 'true_rd', (0.05, 0.075, 0.10)),
                                                  ('E2', 'th2', 'true_rd2', (0.10,))):
                th, true = r[th_key], round(r[true_key], 6)
                it = th.index(true)
                e = a[ep.lower()]
                cov_u, cov_l = n - e['rej_u'][it], n - e['rej_l'][it]
                row = {**base, 'method': m, 'endpoint': ep, 'true': true, 'reps': r['reps'],
                       'valid': n, 'blocks_run': r['blocks_run'],
                       'rd_hat_mean': (r['rd_hat_sum'] if ep == 'E1' else r['rd2_hat_sum']) / n if n else None,
                       'cov_u': cov_u / n if n else None, 'cov_u_class': coverage_class(cov_u, n),
                       'cov_l': cov_l / n if n else None, 'cov_l_class': coverage_class(cov_l, n),
                       'degenerate': e['status'][1], 'failed': e['status'][2], 'na': e['status'][3],
                       'seconds': round(a['seconds'], 2)}
                for d in margins:
                    k = e['rej_u'][th.index(d)]
                    key = f'{d * 1000:g}'
                    row[f'ni_{key}'] = k / n if n else None
                    row[f'ni_{key}_cp_upper'] = cp_upper(k, n) if n else None
                    row[f'inf_{key}'] = e['rej_l'][th.index(d)] / n if n else None
                k0 = e['rej_l'][th.index(0.0)]
                row['worse'] = k0 / n if n else None
                row['worse_cp_upper'] = cp_upper(k0, n) if n else None
                if ep == 'E1':
                    row['overall_ni'] = a['overall_ni'] / n if n else None
                    row['overall_ni_cp_upper'] = cp_upper(a['overall_ni'], n) if n else None
                    row['overall_inf'] = a['overall_inf'] / n if n else None
                    row['true_rd2'] = round(r['true_rd2'], 6)
                yield row


def main(argv=None):
    result_dir, out = argv or sys.argv[1:]
    data = list(rows(result_dir))
    keys = []
    for d in data:
        keys += [k for k in d if k not in keys]
    with open(out, 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=keys)
        w.writeheader()
        w.writerows(data)


if __name__ == '__main__':
    main()
