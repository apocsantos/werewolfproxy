#!/usr/bin/env python3
"""Stage39 M3: Stage22R D07 (target failure) equivalents in the Stage32
sandbox, direct and through a relay (diagnostic regression, not a
certification record).

    d07_sandbox.py BIN_DIR OUT_DIR PATH [PATH ...]

PATH: direct-tcp | direct-quic | routed-tcp | routed-quic (the outer hop).
In one sandbox per path, the D07 series runs sequentially, as in D07:
echo x3, rst x5, kill x5, fin x5, absent x3, with N = 8 MiB of a
64 MiB upload. After every run the receivers' (and relay's) inbound
authority must return to 0 active sessions within CLEANUP_S, and the daemons'
resources are sampled.

Pass rules, fixed before the runs (copied from D07):
  echo        64 MiB echoed exactly, no reset
  rst, kill   a reset seen, exact prefix, not complete
  fin         receive ended with EOF after exactly N bytes, exact prefix
              (how the send direction ended is recorded, not judged)
  absent      a reset seen and 0 bytes received
D07's exclusion: a `kill` run that shows the `fin` pattern (EOF after
exactly N, exact prefix, no reset) is EXCLUDED, not PASS: the dying child's
kernel may have sent FIN, not RST, which is not observable here.
"""
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests' / 'integration'))
import chaos  # noqa: E402

HERE = pathlib.Path(__file__).resolve().parent
N = 8 << 20
TOTAL = 64 << 20
SERIES = [('echo', 3), ('rst', 5), ('kill', 5), ('fin', 5), ('absent', 3)]
CLEANUP_S = 10
D_ADDR = f'{chaos.NODES["b"]}.2'


def verdict(r):
    if 'connect' in r:
        return 'FAIL', 'client could not connect to its Fang'
    mode = r['mode']
    if mode == 'echo':
        ok = r['received'] == TOTAL and r['prefix_exact'] and not r['reset_seen']
        return ('PASS' if ok else 'FAIL'), None
    if mode in ('rst', 'kill'):
        if r['reset_seen'] and r['prefix_exact'] and r['received'] < TOTAL:
            return 'PASS', None
        if (mode == 'kill' and r['recv_end'] == 'eof' and r['received'] == N
                and r['prefix_exact'] and not r['reset_seen']):
            return 'EXCLUDED', 'kill showed the fin pattern (FIN vs RST not observable)'
        return 'FAIL', None
    if mode == 'fin':
        ok = r['recv_end'] == 'eof' and r['received'] == N and r['prefix_exact']
        return ('PASS' if ok else 'FAIL'), None
    if mode == 'absent':
        ok = r['reset_seen'] and r['received'] == 0
        return ('PASS' if ok else 'FAIL'), None
    return 'FAIL', 'unknown mode'


def inside(work, bins, path, out):
    kind, hop = path.split('-')
    routed = kind == 'routed'
    out = pathlib.Path(out)
    box = chaos.Sandbox(pathlib.Path(work), pathlib.Path(bins), None)
    box.build_root()
    nodes = ('a', 'b', 'y') if routed else ('a', 'b')
    box.build_network(nodes)
    ids = {n: box.start(n, n) for n in nodes}
    tdir = box.work / 'd07-target'
    tdir.mkdir()
    target = subprocess.Popen(['nsenter', '--net=/run/netns/b', sys.executable, '-I', '-B',
                               str(HERE / 'd07_target.py'), str(tdir)],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.monotonic() + 15
    while not (tdir / 'ready.json').exists():
        chaos.require(time.monotonic() < deadline, 'target not ready')
        time.sleep(.1)
    ports = json.loads((tdir / 'ready.json').read_text())
    box.pack_add('a', 'wolf-b', ids['b'], 'b')
    box.pack_add('b', 'wolf-a', ids['a'], 'a')
    if routed:
        box.pack_add('a', 'wolf-y', ids['y'], 'y')
        box.pack_add('y', 'wolf-a', ids['a'], 'a')
        box.ctl('y', 'target.allow', {'peer': 'wolf-a', 'target': f'{D_ADDR}:{chaos.PORT}'})
        box.ctl('a', 'route.set', {'peer': 'wolf-b', 'via': 'wolf-y', 'hop': hop})
    for p in (ports['port'], ports['absent']):
        box.ctl('b', 'target.allow', {'peer': 'wolf-a', 'target': f'127.0.0.1:{p}'})
    transport = 'quic' if routed else hop
    fangs = {'live': box.fang('a', 'wolf-b', '127.0.0.1:17001', f'127.0.0.1:{ports["port"]}', transport),
             'absent': box.fang('a', 'wolf-b', '127.0.0.1:17002', f'127.0.0.1:{ports["absent"]}', transport)}
    time.sleep(2)
    receivers = ('b', 'y') if routed else ('b',)
    baseline = {n: box.sample(n) for n in nodes}
    runs = []
    for mode, count in SERIES:
        for i in range(1, count + 1):
            local = '127.0.0.1:17002' if mode == 'absent' else '127.0.0.1:17001'
            out_ = chaos.sh('nsenter', '--net=/run/netns/a', sys.executable, '-I', '-B',
                            str(HERE / 'd07_client.py'), local, mode, str(N), str(TOTAL),
                            check=False)
            try:
                r = json.loads(out_.stdout.strip().splitlines()[-1])
            except (IndexError, json.JSONDecodeError):
                r = {'mode': mode, 'connect': 'no output', 'stderr': out_.stderr[-500:]}
            r['run'] = f'{mode}-{i}'
            t0 = time.monotonic()
            while True:
                active = {n: (box.try_ctl(n, 'status').get('inbound_authority') or {}).get('active')
                          for n in receivers}
                if all(v == 0 for v in active.values()) or time.monotonic() - t0 > CLEANUP_S:
                    break
                time.sleep(.2)
            r['leases_after'] = active
            r['lease_release_s'] = round(time.monotonic() - t0, 2)
            r['resources'] = {n: box.sample(n) for n in nodes}
            r['verdict'], r['note'] = verdict(r)
            if any(v != 0 for v in active.values()):
                r['verdict'], r['note'] = 'FAIL', f'lease not released within {CLEANUP_S} s'
            runs.append(r)
            time.sleep(1)
    time.sleep(chaos.SETTLE_S)
    after = {n: box.sample(n) for n in nodes}
    leak = {n: {k: after[n][k] - baseline[n][k] for k in ('fds', 'threads', 'rss_kib')} for n in nodes}
    leak_ok = all(leak[n]['fds'] <= chaos.LEAK['fds'] and leak[n]['threads'] <= chaos.LEAK['threads']
                  and leak[n]['rss_kib'] <= chaos.LEAK['rss_kib'] and after[n]['alive'] for n in nodes)
    (out / f'{path}.json').write_text(json.dumps(
        {'path': path, 'fangs': fangs, 'ports': ports, 'baseline': baseline, 'after_settle': after,
         'leak_delta': leak, 'leak_ok': leak_ok, 'runs': runs}, indent=1))
    for process in box.daemons.values():
        process.terminate()
    target.terminate()


def main():
    if sys.argv[1] == '--inside':
        return inside(*sys.argv[2:6])
    bins, out, *paths = sys.argv[1:]
    os.makedirs(out, exist_ok=True)
    os.chmod(out, 0o755)
    for path in paths:
        work = tempfile.mkdtemp(prefix='wwp-d07-', dir=str(ROOT / 'target' / 'stage39-m3'))
        os.chmod(work, 0o755)
        started = time.time()
        rc = subprocess.run(['unshare', '-rmnp', '--fork', '--kill-child', '--', sys.executable,
                             '-B', __file__, '--inside', work, bins, path, out]).returncode
        print(json.dumps({'path': path, 'rc': rc, 'seconds': round(time.time() - started)}), flush=True)
        subprocess.run(['rm', '-rf', work])


if __name__ == '__main__':
    main()
