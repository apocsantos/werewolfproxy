#!/usr/bin/env python3
"""Stage49 M1: a QUIC peer with nothing listening (disposable lab).

Runs in the Stage46 sandbox (tools/stage46/dial.py) with the Stage48 lab
and names (tools/stage48/retry.py): `refused.stage46` (127.0.0.2, nothing
listens), `blackhole.stage46` (10.46.0.9, dropped), and Stage46's
`fast.stage46` (127.0.0.2, then 127.0.0.1).

Scenarios are <path>:<shape>, <path> one of:

  quic, tcp                  A's direct Fang to B (Stage48)
  failover-quic,             providers B by name, then C live; or B and C
  failover3-quic             by name, then D live (Stage48)
  routed-quic, routed-tcp    A's Fang to C via relay B by name (Stage46)
  discovery-quic,            A's `discover refresh` of B by name (Stage46)
  discovery-tcp
  sync-quic, sync-tcp        B's `governance sync` from A by name (Stage46)

and one more, without a shape:

  datagrams                  A's QUIC Fang to B (literal): 3 clients, then
                             --datagrams 1-byte UDP datagrams to B's QUIC
                             port, then 3 more clients. Records B's log
                             growth and CPU during the datagrams, whether
                             every client echoed, and B's `fang.list`.

Prints one JSON line per scenario; writes all to --out.
"""

import argparse
import json
import os
import pathlib
import socket
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/stage48'))
sys.dont_write_bytecode = True
import retry  # noqa: E402  (also imports tools/stage46/dial.py and the Stage45 lab)
from retry import RetryLab, dial  # noqa: E402
from visibility import LogCap  # noqa: E402

SHAPES = ('refused', 'blackhole', 'fast')
RETRY_PATHS = ('quic', 'tcp', 'failover-quic', 'failover3-quic')
DIAL_PATHS = ('routed-quic', 'routed-tcp')
CLK_TCK = os.sysconf('SC_CLK_TCK')


def cpu_ticks(pid):
    fields = pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return int(fields[11]) + int(fields[12])


def b_log(lab):
    return b''.join(p.read_bytes() for p in sorted(lab.base.glob('*-b-*.log')))


def scenario_datagrams(lab, count, attempts):
    lab.boot()
    result = {'scenario': 'datagrams', 'datagrams': count}
    for wolf in ('b', 'c', 'd'):
        lab.pack_literal(wolf, 'a')
    lab.pack_literal('a', 'b')
    lab.ctl('a', 'service', 'add', 'echo', 'wolf-b', lab.echo, 'tcp')
    lab.ctl('b', 'target', 'allow', 'wolf-a', lab.echo)
    opened = lab.connect('quic')
    result['open_exit'] = opened['exit']
    if opened['exit']:
        result['open'] = {'body': opened['body'], 'stderr': opened['stderr']}
        return result
    result['before'] = lab.measure_timed(attempts)['clients']
    b_pid = lab.wolves[lab.WOLVES.index('b')].pid
    log_before = len(b_log(lab))
    ticks_before = cpu_ticks(b_pid)
    started = time.monotonic()
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.connect(('127.0.0.1', lab.ports['b_quic']))
    sent = refused = 0
    for _ in range(count):
        try:
            sock.send(b'x')
            sent += 1
        except ConnectionRefusedError:
            refused += 1
    sock.close()
    time.sleep(1)
    result['sent'] = sent
    result['refused_by_kernel'] = refused
    result['send_seconds'] = round(time.monotonic() - started - 1, 2)
    result['b_cpu_seconds'] = round((cpu_ticks(b_pid) - ticks_before) / CLK_TCK, 2)
    growth = b_log(lab)[log_before:].decode(errors='replace').splitlines()
    result['b_log_lines'] = len(growth)
    result['b_log_sample'] = [line[:240] for line in growth[:3]]
    after = lab.measure_timed(attempts)
    result['after'] = after['clients']
    result['echo'] = sum(1 for o, _ in result['before'] + result['after'] if o == 'echo')
    result['fang_list'] = after['fang_list']
    result['daemons_alive'] = after['daemons_alive']
    return result


def run(lab, item, attempts, datagrams):
    if item == 'datagrams':
        return scenario_datagrams(lab, datagrams, attempts)
    path, _, shape = item.partition(':')
    if path in RETRY_PATHS:
        return retry.scenario(lab, path, shape, attempts)
    if path in DIAL_PATHS:
        return dial.scenario_fang(lab, path, shape, attempts)
    if path.startswith('discovery-'):
        return dial.scenario_discovery(lab, path.split('-')[1], shape)
    return dial.scenario_sync(lab, path.split('-')[1], shape)


def valid(item):
    if item == 'datagrams':
        return True
    path, _, shape = item.partition(':')
    known = RETRY_PATHS + DIAL_PATHS + ('discovery-quic', 'discovery-tcp', 'sync-quic', 'sync-tcp')
    return path in known and shape in SHAPES


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('scenarios', nargs='+', help='e.g. quic:refused sync-quic:fast datagrams')
    parser.add_argument('--attempts', type=int, default=3)
    parser.add_argument('--datagrams', type=int, default=1000)
    parser.add_argument('--out', required=True)
    parser.add_argument('--keep-temp', action='store_true')
    args = parser.parse_args()
    for item in args.scenarios:
        if not valid(item):
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
        os.umask(0o077)
        lab = RetryLab(args.keep_temp)
        try:
            result = run(lab, item, args.attempts, args.datagrams)
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
