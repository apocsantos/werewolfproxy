#!/usr/bin/env python3
"""Stage39 M1: summarise relay_target_liveness.py records.

    analyse.py RUNS_DIR            prints a Markdown table and writes
                                   RUNS_DIR/summary.json

Times are seconds after fault injection. A run is INVALID unless the
session was established, the sockets were identified, A could reach Y in
every sample after the fault, and (with a fault) Y could not reach D.
"""
import json
import pathlib
import sys


def first(samples, pred, t_fault):
    for s in samples:
        if s['t'] >= t_fault and pred(s):
            return round(s['t'] - t_fault, 1)
    return None


def summarise(r):
    tf = r['t_fault']
    ts = r['t_stop']
    after = [s for s in r['samples'] if s['t'] >= tf]
    window = [s for s in after if ts is None or s['t'] < ts]
    post = [s for s in after if ts is not None and s['t'] >= ts]
    bound = r['declared']['bound_s']
    at_bound = next((s for s in window if s['t'] - tf >= bound), None)
    active = lambda s: (s['auth_y'] or {}).get('active')
    valid = (r['session_ready'] and r['identity_valid'] and all(s['a_to_y'] for s in window)
             and (r['fault'] == 'none' or all(not s['y_to_d'] for s in after[1:])))
    timers = sorted({(x.get('timer') or 'none').split(',')[0] for s in window for x in s['y_target']})
    max_unacked = max((int(x.get('unacked', 0)) for s in window for x in s['y_target']), default=0)
    max_backoff = max((int(x.get('backoff', 0)) for s in window for x in s['y_target']), default=0)
    # Client events carry absolute CLOCK_MONOTONIC times; the driver's sample
    # clock is relative. The client logs 'stop' within 0.2 s of the driver's
    # stop marker (written at t_stop), which anchors the two clocks (+-1 s).
    stop_ev = next((e for e in r['client'] if e['ev'] == 'stop'), None)
    offset = stop_ev['t'] - ts if stop_ev and ts is not None else None
    client = [(e['ev'], e.get('kind'), round(e['t'] - offset - tf, 1) if offset is not None else None)
              for e in r['client'] if e['ev'] in ('eof', 'error')]
    out = {
        'scenario': r['scenario'], 'valid': valid,
        'y_target_opts': {k: r['sockopts'].get('y_target', {}).get(k) for k in
                          ('SO_KEEPALIVE', 'TCP_USER_TIMEOUT', 'TCP_NODELAY')},
        'y_target_gone_s': first(after, lambda s: not s['y_target'], tf),
        'y_target_state_at_bound': [x['state'] for x in at_bound['y_target']] if at_bound else None,
        'y_target_timers': timers, 'y_target_max_unacked': max_unacked,
        'y_target_max_backoff': max_backoff,
        'd_accepted_gone_s': first(after, lambda s: not s['d_accepted'], tf),
        'y_active_at_fault': active(after[0]) if after else None,
        'y_active_zero_s': first(after, lambda s: active(s) == 0, tf),
        'y_active_at_bound': active(at_bound) if at_bound else None,
        'y_active_end_of_window': active(window[-1]) if window else None,
        'y_active_end_post': active(post[-1]) if post else None,
        'y_target_end_post': [x['state'] for x in post[-1]['y_target']] if post else None,
        'client_end': client,
        'fds': (r['baseline']['y']['fds'], r['after']['y']['fds']),
    }
    if r['fault'] == 'none':
        out['class'] = 'CONTROL'
    elif not valid:
        out['class'] = 'INVALID'
    elif out['y_active_at_bound'] and out['y_target_state_at_bound']:
        out['class'] = 'PERSISTED_BEYOND_BOUND'
    else:
        out['class'] = 'RELEASED_WITHIN_BOUND'
    return out


def main():
    runs = pathlib.Path(sys.argv[1])
    rows = [summarise(json.loads(p.read_text())) for p in sorted(runs.glob('*.json'))
            if p.name != 'summary.json']
    (runs / 'summary.json').write_text(json.dumps(rows, indent=1))
    cols = ('scenario', 'class', 'y_target_gone_s', 'y_target_state_at_bound', 'y_target_timers',
            'y_target_max_unacked', 'y_target_max_backoff', 'd_accepted_gone_s', 'y_active_zero_s',
            'y_active_at_bound', 'y_active_end_post', 'y_target_end_post', 'client_end', 'fds')
    print('| ' + ' | '.join(cols) + ' |')
    print('|' + '---|' * len(cols))
    for r in rows:
        print('| ' + ' | '.join(str(r[c]) for c in cols) + ' |')


if __name__ == '__main__':
    main()
