"""Small summary tables for Git from the full per-scenario CSVs.

    python -m s38.summarise TARGET_DIR RESULTS_DIR

TARGET_DIR holds screening.csv, power.csv, confirmation.csv and interim
result JSONs; RESULTS_DIR receives the summaries and SHA256SUMS of the full
CSVs (which stay outside Git).
"""
import glob
import hashlib
import json
import os
import sys

import pandas as pd

from .analysis import cp_upper

MARGINS = ((0.05, 'ni_50'), (0.075, 'ni_75'), (0.10, 'ni_100'))


def _read(path):
    d = pd.read_csv(path, low_memory=False)
    skipped = int(d['skipped'].notna().sum()) if 'skipped' in d else 0
    return d[d['method'].notna()].copy(), skipped


def error_table(d):
    out = []
    for m, g in d.groupby('method'):
        row = {'method': m}
        for dl, k in MARGINS:
            nul = g[g.true_rd >= dl - 1e-9]
            row[f'null_cells_{k}'] = len(nul)
            row[f'max_fa_{k}'] = round(nul[k].max(), 4)
            row[f'max_fa_cp_upper_{k}'] = round(nul[k + '_cp_upper'].max(), 4)
            row[f'cells_cp_upper_gt_5pct_{k}'] = int((nul[k + '_cp_upper'] > 0.05).sum())
            row[f'cells_clearly_gt_5pct_{k}'] = int((nul[k + '_cp_lower'] > 0.05).sum())
        row['min_cov_u'] = round(g.cov_u.min(), 4)
        row['cells_cov_u_cp_upper_lt_95'] = int((g.cov_u_cp_upper < 0.95).sum())
        row['min_cov_l'] = round(g.cov_l.min(), 4)
        row['cells_cov_l_cp_upper_lt_95'] = int((g.cov_l_cp_upper < 0.95).sum())
        zero = g[g.true_rd <= 0]
        row['max_false_worsening'] = round(zero.worse.max(), 4)
        row['degenerate'] = int(g.status_degenerate.sum())
        row['failed'] = int(g.status_failed.sum())
        row['na'] = int(g.status_na.sum())
        row['replicates'] = int(g.valid.sum())
        out.append(row)
    return pd.DataFrame(out)


def by_factor(d, factor, k='ni_75', dl=0.075):
    nul = d[(d.true_rd - dl).abs() < 1e-9]
    return nul.pivot_table(index=factor, columns='method', values=k, aggfunc='max').round(4)


def sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def main(argv=None):
    target, results = (argv or sys.argv[1:])
    os.makedirs(results, exist_ok=True)
    sums = []
    notes = {}
    for stage in ('screening', 'power', 'confirmation', 'interim'):
        path = os.path.join(target, stage + '.csv')
        if not os.path.exists(path):
            continue
        sums.append(f'{sha(path)}  {stage}.csv')
        d, skipped = _read(path)
        notes[stage] = {'scenario_rows': int(len(d)), 'skipped_infeasible': skipped}
        if stage == 'interim':
            continue
        error_table(d).to_csv(os.path.join(results, f'{stage}_error_control.csv'), index=False)
        for factor in ('sigma', 'p_a', 'quartets', 'group', 'asym', 'phi', 'seeds', 'drift', 'p_invalid'):
            if factor in d and d[factor].nunique() > 1:
                by_factor(d, factor).to_csv(os.path.join(results, f'{stage}_max_fa75_by_{factor}.csv'))
        if stage == 'power':
            for dl, k in MARGINS:
                p = d[(d.method == 'C6') & (d.true_rd == 0)].pivot_table(
                    index=['p_a', 'sigma'], columns='quartets', values=k).round(4)
                p.to_csv(os.path.join(results, f'power_C6_{k}.csv'))
    # Interim: from the raw JSONs (counts per alpha).
    rows = []
    for path in sorted(glob.glob(os.path.join(target, 'interim', '[0-9]*.json'))):
        with open(path) as f:
            r = json.load(f)
        if 'interim' not in r:
            continue
        th = r['thresholds']
        for m, a in r['interim'].items():
            for alpha, c in a.items():
                n = r['valid']
                ni_final = r['methods'][m]['rej_u'][th.index(0.075)]
                rows.append({**r['cell'], 'method': m, 'alpha_interim': float(alpha), 'valid': n,
                             'stop_rate': c['stop'] / n, 'stop_cp_upper': cp_upper(c['stop'], n),
                             'ni_without_interim': ni_final / n,
                             'ni_with_interim': c['ni_not_stopped'] / n,
                             'stopped_but_final_ni': c['stop_and_ni'] / n})
    if rows:
        pd.DataFrame(rows).round(5).to_csv(os.path.join(results, 'interim.csv'), index=False)
    with open(os.path.join(results, 'SHA256SUMS'), 'w') as f:
        f.write('\n'.join(sums) + '\n')
    with open(os.path.join(results, 'notes.json'), 'w') as f:
        json.dump(notes, f, indent=1)


if __name__ == '__main__':
    main()
