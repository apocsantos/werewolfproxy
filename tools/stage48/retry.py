#!/usr/bin/env python3
"""Stage48 M1: the encrypted-TCP setup retry against a peer that does not
answer (disposable lab).

Runs in the Stage46 sandbox (tools/stage46/dial.py: a private user, mount
and network namespace, a private /etc/hosts and /etc/gai.conf, and a
`dummy` interface that silently drops 10.46.0.0/24), with two more names:

  refused     127.0.0.2 (nothing listens: TCP refused)
  blackhole   10.46.0.9 (dropped silently)
  dead        127.0.0.2, then 10.46.0.9 (Stage46)

Scenarios are <path>:<shape>, <path> one of:

  tcp, quic              A's direct Fang to B, B's Pack address on A
                         <scheme>://<shape>.stage46:PORT
  failover-tcp,          providers B (address by name, over the scheme)
  failover-quic          then C (literal, QUIC, answers)
  failover3-tcp,         providers B and C (both by name, over the scheme)
  failover3-quic         then D (literal, QUIC, answers)

Each client (one at a time) records its outcome and the seconds to it,
with a 30 s client timeout so that the daemon's 15 s selection window,
not the client, ends a failover attempt. Also A's log lines and A's
`fang.list` entry. Prints one JSON line per scenario; writes all to --out.
"""

import argparse
import json
import os
import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/stage46'))
sys.dont_write_bytecode = True
import dial  # noqa: E402
from visibility import LogCap, attempt  # noqa: E402

dial.SHAPES.update({'refused': ['127.0.0.2'], 'blackhole': ['10.46.0.9']})
SHAPES = ('refused', 'blackhole', 'dead')
PATHS = ('tcp', 'quic', 'failover-tcp', 'failover-quic', 'failover3-tcp', 'failover3-quic')
CLIENT_TIMEOUT = 30


class RetryLab(dial.DialLab):
    WOLVES = ('a', 'b', 'c', 'd')

    def measure_timed(self, count):
        before = len(self.a_log())
        outcomes = []
        for _ in range(count):
            self.guard()
            start = time.monotonic()
            outcome = attempt(self.ports['fang'], timeout=CLIENT_TIMEOUT)
            outcomes.append([outcome, round(time.monotonic() - start, 2)])
        time.sleep(1)
        lines = self.a_log()[before:].decode(errors='replace').splitlines()
        return {'clients': outcomes,
                'echo': sum(1 for o, _ in outcomes if o == 'echo'),
                'a_log_lines': len(lines),
                'a_log_sample': [line[:300] for line in lines[:4]],
                'fang_list': self.fang_list(),
                'daemons_alive': all(w.poll() is None for w in self.wolves)}


def scenario(lab, path, shape, attempts):
    lab.boot()
    result = {'scenario': f'{path}:{shape}', 'resolved': dial.resolved(shape)}
    for wolf in ('b', 'c', 'd'):
        lab.pack_literal(wolf, 'a')
    kind, _, scheme = path.rpartition('-') if '-' in path else ('', '', path)
    if not kind:
        result['pack_add'] = lab.pack_name('a', 'b', shape, scheme)['exit']
        lab.ctl('a', 'service', 'add', 'echo', 'wolf-b', lab.echo, 'tcp')
        lab.ctl('b', 'target', 'allow', 'wolf-a', lab.echo)
        return dial.open_and_measure(lab, scheme, attempts, result)
    silent = ['b'] if kind == 'failover' else ['b', 'c']
    live = 'c' if kind == 'failover' else 'd'
    result['pack_add'] = [lab.pack_name('a', w, shape, scheme)['exit'] for w in silent]
    lab.pack_literal('a', live)
    lab.ctl('a', 'service', 'add', 'echo', 'wolf-b', lab.echo, 'tcp')
    for wolf in silent[1:] + [live]:
        lab.ctl('a', 'service', 'provider', 'add', 'echo', f'wolf-{wolf}', lab.echo)
    for wolf in silent + [live]:
        lab.ctl(wolf, 'target', 'allow', 'wolf-a', lab.echo)
    return dial.open_and_measure(lab, scheme, attempts, result)


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('scenarios', nargs='+', help='e.g. tcp:blackhole failover3-tcp:dead')
    parser.add_argument('--attempts', type=int, default=3)
    parser.add_argument('--out', required=True)
    parser.add_argument('--keep-temp', action='store_true')
    args = parser.parse_args()
    for item in args.scenarios:
        path, _, shape = item.partition(':')
        if path not in PATHS or shape not in SHAPES:
            raise SystemExit(f'unknown scenario {item}')
    out = pathlib.Path(args.out).resolve()
    if os.environ.get('WWP_STAGE46_SANDBOX') != '1':
        env = dict(os.environ, WWP_STAGE46_SANDBOX='1')
        os.execvpe('unshare', ['unshare', '-rmn', '--fork', '--kill-child', '--',
                               sys.executable, '-B', str(pathlib.Path(__file__).resolve()),
                               *sys.argv[1:]], env)
    dial.enter_sandbox()

    results = []
    for item in args.scenarios:
        path, _, shape = item.partition(':')
        os.umask(0o077)
        lab = RetryLab(args.keep_temp)
        try:
            result = scenario(lab, path, shape, args.attempts)
        except LogCap as exc:
            result = {'scenario': item, 'log_cap': str(exc)}
        except Exception as exc:  # record and continue with the next scenario
            result = {'scenario': item, 'error': f'{type(exc).__name__}: {exc}'}
        finally:
            result['daemon_sha256'] = getattr(lab, 'binary_hash', None)
            try:
                lab.cleanup()
            except Exception as exc:
                result['cleanup_error'] = str(exc)
        print(json.dumps(result), flush=True)
        results.append(result)
    out.write_text(json.dumps(results, indent=1) + '\n')


if __name__ == '__main__':
    main()
