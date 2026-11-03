"""Small M3b summary tables for Git.

    python -m s38.m3b_summarise TARGET_DIR RESULTS_DIR

TARGET_DIR (target/stage38-m3b) holds <stage>.csv from s38.m3b_analysis;
RESULTS_DIR receives m3b_*.csv summaries and m3b_SHA256SUMS.
"""
import hashlib
import os
import sys

import pandas as pd

STAGES = ('screening', 'power', 'confirmation', 'hierarchy', 'replacement', 'r1_null')


def _sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def error_control(d):
    e1 = d[d.endpoint == 'E1']
    out = []
    for (m, g), x in e1.groupby(['method', 'group']):
        row = {'method': m, 'family': g, 'cells': len(x)}
        for dl, k in ((0.05, 'ni_50'), (0.075, 'ni_75'), (0.10, 'ni_100')):
            nul = x[x.true >= dl - 1e-9]
            row[f'max_fa_{k}'] = round(nul[k].max(), 5) if len(nul) else None
            row[f'max_fa_cp_upper_{k}'] = round(nul[k + '_cp_upper'].max(), 5) if len(nul) else None
            row[f'cells_cp_upper_gt_5pct_{k}'] = int((nul[k + '_cp_upper'] > 0.05).sum())
        for b in ('u', 'l'):
            vc = x[f'cov_{b}_class'].value_counts()
            for c in ('adequate', 'inconclusive', 'under'):
                row[f'cov_{b}_{c}'] = int(vc.get(c, 0))
        row['max_false_worsening'] = round(x[x.true <= 0].worse.max(), 5) if (x.true <= 0).any() else None
        row['degenerate'] = int(x.degenerate.sum())
        row['failed'] = int(x.failed.sum())
        out.append(row)
    return pd.DataFrame(out)


def main(argv=None):
    target, results = argv or sys.argv[1:]
    os.makedirs(results, exist_ok=True)
    sums = []
    for stage in STAGES:
        path = os.path.join(target, stage + '.csv')
        if not os.path.exists(path):
            continue
        sums.append(f'{_sha(path)}  {stage}.csv')
        d = pd.read_csv(path, low_memory=False)
        d = d[d['method'].notna()]
        error_control(d).to_csv(os.path.join(results, f'm3b_{stage}_error_control.csv'), index=False)
        e1 = d[(d.endpoint == 'E1') & (d.true == 0)]
        if stage in ('screening', 'power', 'confirmation') and len(e1):
            e1.pivot_table(index=['group', 'p_a', 'sigma'], columns=['method', 'quartets'],
                           values='ni_75', aggfunc='min').round(4).to_csv(
                os.path.join(results, f'm3b_{stage}_power_e1_d75.csv'))
        if stage == 'hierarchy':
            h = d[d.endpoint == 'E1'][['group', 'p_a', 'sigma', 'lam2', 'quartets', 'method', 'true',
                                       'true_rd2', 'e2_a', 'valid', 'ni_75', 'overall_ni',
                                       'overall_ni_cp_upper', 'overall_inf']]
            h.round(5).to_csv(os.path.join(results, 'm3b_hierarchy.csv'), index=False)
        if stage == 'replacement':
            r = d[(d.endpoint == 'E1')].copy()
            r['completion'] = r.valid / r.reps
            r['blocks_per_valid'] = r.blocks_run / r.reps / (4 * r.quartets)
            r[['invalid', 'policy', 'quartets', 'true', 'method', 'completion', 'blocks_per_valid',
               'rd_hat_mean', 'ni_75', 'ni_75_cp_upper', 'cov_u_class', 'cov_l_class']].round(5).to_csv(
                os.path.join(results, 'm3b_replacement.csv'), index=False)
    with open(os.path.join(results, 'm3b_SHA256SUMS'), 'w') as f:
        f.write('\n'.join(sums) + '\n')


if __name__ == '__main__':
    main()
