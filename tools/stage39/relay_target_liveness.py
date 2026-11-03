#!/usr/bin/env python3
"""Stage39 M1: relay-to-destination liveness reproduction (diagnostic only).

Runs the real Stage26 routed Fang A -> Y -> D in the Stage32 sandbox
(tests/integration/chaos.py: unprivileged user, network and PID namespaces,
a private root, disposable Dens), on unchanged production binaries, and
observes what happens on Y when the Y <-> D path fails while A <-> Y stays
healthy. Nothing here changes production code or any Stage32 rule.

    relay_target_liveness.py BIN_DIR OUT_DIR SCENARIO [SCENARIO ...]

SCENARIO is HOP:FAULT:ACTIVITY:REP, for example tcp:blackhole:idle:1.
  HOP       tcp | quic        the outer A -> Y hop (route.set --hop)
  FAULT     none | blackhole | linkdown
              blackhole: netem loss 100% on both ends of D's link only
                         (D's link carries only Y <-> D traffic here);
                         packets vanish silently.
              linkdown:  the router-side end of D's link set down; the
                         router has no route to D, so Y's packets meet a
                         routing error (ICMP unreachable), not silence.
  ACTIVITY  idle | upload | download

Pre-declared (before any run): fault injected FAULT_AT_S after the
session is established and carrying its activity; observed for WINDOW_S;
anything on Y still present BOUND_S after the fault is "beyond bound"
(the session sockets' liveness bound: keep-alive 30 s + 3 x 10 s, or
TCP_USER_TIMEOUT 60 s, plus CLEANUP_WINDOW 5 s, with margin). Then the
client closes and Y is observed for POST_S more.
"""
import ctypes
import json
import os
import pathlib
import re
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests' / 'integration'))
import chaos  # noqa: E402

HERE = pathlib.Path(__file__).resolve().parent
FAULT_AT_S = 5
WINDOW_S = 300
BOUND_S = 150
POST_S = 90
if os.environ.get('S39_SMOKE'):          # topology check only; never evidence
    WINDOW_S, POST_S = 20, 5
D_ADDR = f'{chaos.NODES["b"]}.2'
Y_ADDR = f'{chaos.NODES["y"]}.2'
A_ADDR = f'{chaos.NODES["a"]}.2'

# Socket options read from a duplicate of the daemon's descriptor.
SOL_TCP, SOL_SOCKET = 6, 1
OPTS = {'SO_KEEPALIVE': (SOL_SOCKET, 9), 'TCP_KEEPIDLE': (SOL_TCP, 4),
        'TCP_KEEPINTVL': (SOL_TCP, 5), 'TCP_KEEPCNT': (SOL_TCP, 6),
        'TCP_USER_TIMEOUT': (SOL_TCP, 18), 'TCP_NODELAY': (SOL_TCP, 1)}
_libc = ctypes.CDLL(None, use_errno=True)


def sockopts(pid, fd):
    """getsockopt on a pidfd_getfd duplicate: the daemon's socket is only
    read, never changed; closing the duplicate leaves the original open."""
    pidfd = os.pidfd_open(pid)
    try:
        dup = _libc.syscall(438, pidfd, fd, 0)          # pidfd_getfd
        if dup < 0:
            return {'error': os.strerror(ctypes.get_errno())}
        s = socket.socket(fileno=dup)
        try:
            out = {k: s.getsockopt(level, opt) for k, (level, opt) in OPTS.items()}
            onoff, linger = struct.unpack('ii', s.getsockopt(SOL_SOCKET, 13, 8))   # SO_LINGER
            out['SO_LINGER'] = {'onoff': onoff, 'linger': linger}
            return out
        finally:
            s.close()
    finally:
        os.close(pidfd)


SS_LINE = re.compile(r'^(\S+)\s+(\d+)\s+(\d+)\s+(\S+)\s+(\S+)(.*)$')


def tcp_sockets(box, ns):
    """Every TCP socket in a namespace (ss -tinopeH, all states)."""
    out = box.in_net(ns, 'ss', '-tinopeHa', check=False).stdout
    socks, cur = [], None
    for line in out.splitlines():
        m = SS_LINE.match(line)
        if m and not line.startswith('\t'):
            cur = {'state': m[1], 'recvq': int(m[2]), 'sendq': int(m[3]), 'local': m[4],
                   'peer': m[5], 'rest': m[6].strip()}
            pm = re.search(r'pid=(\d+),fd=(\d+)', m[6])
            if pm:
                cur['pid'], cur['fd'] = int(pm[1]), int(pm[2])
            tm = re.search(r'timer:\(([^)]*)\)', m[6])
            cur['timer'] = tm[1] if tm else None
            socks.append(cur)
        elif cur is not None:
            info = line.strip()
            for key in ('unacked', 'retrans', 'backoff', 'lastsnd', 'lastrcv', 'lastack',
                        'bytes_acked', 'bytes_received', 'bytes_sent', 'rto', 'notsent'):
                km = re.search(rf'\b{key}:([\d./]+)', info)
                if km:
                    cur[key] = km[1]
    return socks


def pick(socks, local=None, peer=None):
    out = []
    for s in socks:
        if s['state'] in ('LISTEN', 'UNCONN'):
            continue
        if local and not s['local'].startswith(local):
            continue
        if peer and not s['peer'].startswith(peer):
            continue
        out.append(s)
    return out


def ping(box, ns, addr):
    return box.in_net(ns, 'ping', '-n', '-c', '1', '-W', '1', addr, check=False).returncode == 0


def inside(work, bins, scenario, out):
    hop, fault, activity, rep = scenario.split(':')
    out = pathlib.Path(out)
    record = {'scenario': scenario, 'hop': hop, 'fault': fault, 'activity': activity,
              'rep': int(rep), 'declared': {'fault_at_s': FAULT_AT_S, 'window_s': WINDOW_S,
                                            'bound_s': BOUND_S, 'post_s': POST_S}}
    box = chaos.Sandbox(pathlib.Path(work), pathlib.Path(bins), None)
    box.build_root()
    nodes = ('a', 'b', 'y')
    box.build_network(nodes)
    ids = {n: box.start(n, n) for n in nodes}
    pids = {n: box.daemons[n].pid for n in nodes}
    tdir = box.work / 's39-targets'
    tdir.mkdir()
    targets = subprocess.Popen(['nsenter', '--net=/run/netns/b', sys.executable, '-I', '-B',
                                str(HERE / 's39_targets.py'), str(tdir)],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.monotonic() + 15
    while not (tdir / 'ready.json').exists():
        chaos.require(time.monotonic() < deadline, 'targets not ready')
        time.sleep(.1)
    ports = json.loads((tdir / 'ready.json').read_text())
    # The Stage26 routed configuration, exactly as chaos.transfers sets it up.
    box.pack_add('a', 'wolf-b', ids['b'], 'b')
    box.pack_add('a', 'wolf-y', ids['y'], 'y')
    box.pack_add('b', 'wolf-a', ids['a'], 'a')
    box.pack_add('y', 'wolf-a', ids['a'], 'a')
    box.ctl('y', 'target.allow', {'peer': 'wolf-a', 'target': f'{D_ADDR}:{chaos.PORT}'})
    port = ports[{'idle': 'echo', 'upload': 'sink', 'download': 'source'}[activity]]
    box.ctl('b', 'target.allow', {'peer': 'wolf-a', 'target': f'127.0.0.1:{port}'})
    box.ctl('a', 'route.set', {'peer': 'wolf-b', 'via': 'wolf-y', 'hop': hop})
    record['fang'] = box.fang('a', 'wolf-b', '127.0.0.1:17001', f'127.0.0.1:{port}', 'quic')
    time.sleep(2)
    record['baseline'] = {n: box.sample(n) for n in nodes}
    record['baseline_authority'] = {n: box.ctl(n, 'status')['inbound_authority'] for n in ('y', 'b')}

    log = box.work / 'client.jsonl'
    stop = box.work / 'client.stop'
    client = subprocess.Popen(['nsenter', '--net=/run/netns/a', sys.executable, '-I', '-B',
                               str(HERE / 's39_client.py'), '127.0.0.1:17001', activity,
                               str(log), str(stop)],
                              stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    events = lambda: [json.loads(l) for l in log.read_text().splitlines()] if log.exists() else []
    deadline = time.monotonic() + 30
    ready = False
    while time.monotonic() < deadline and not ready:
        ev = events()
        if activity == 'idle':
            ready = any(e['ev'] == 'probe' for e in ev)
        else:
            prog = [e for e in ev if e['ev'] == 'progress']
            ready = bool(prog) and max(prog[-1]['sent'], prog[-1]['received']) > (1 << 20)
        time.sleep(.2)
    record['session_ready'] = ready
    record['probe_ok'] = next((e['ok'] for e in events() if e['ev'] == 'probe'), None)

    # Socket identity: Y's target connection to D, D's accepted socket from Y,
    # and (TCP hop) Y's accepted outer socket from A.
    ysocks, bsocks = tcp_sockets(box, 'y'), tcp_sockets(box, 'b')
    target = pick(ysocks, local=Y_ADDR, peer=f'{D_ADDR}:{chaos.PORT}')
    d_accepted = pick(bsocks, local=f'{D_ADDR}:{chaos.PORT}', peer=Y_ADDR)
    outer = pick(ysocks, local=f'{Y_ADDR}:{chaos.PORT}', peer=A_ADDR)
    record['identity'] = {'y_target': target, 'd_accepted': d_accepted, 'y_outer': outer}
    record['sockopts'] = {}
    for name, socks in (('y_target', target), ('d_accepted', d_accepted), ('y_outer', outer)):
        if len(socks) == 1 and 'fd' in socks[0]:
            record['sockopts'][name] = sockopts(socks[0]['pid'], socks[0]['fd'])
    record['identity_valid'] = len(target) == 1 and len(d_accepted) == 1 and \
        target[0].get('pid') == pids['y'] and (hop == 'quic' or len(outer) == 1)

    samples = []
    t0 = time.monotonic()
    t_fault = None
    stopped_at = None
    fault_cmds = []
    while True:
        now = time.monotonic() - t0
        if t_fault is None and now >= FAULT_AT_S:
            if fault == 'blackhole':
                for ns, dev in (('b', 'vb'), ('r', 'rb')):
                    cmd = ['/usr/sbin/tc', 'qdisc', 'add', 'dev', dev, 'root', 'netem', 'loss', '100%']
                    box.in_net(ns, *cmd)
                    fault_cmds.append({'ns': ns, 'cmd': cmd})
            elif fault == 'linkdown':
                cmd = ['ip', 'link', 'set', 'rb', 'down']
                box.in_net('r', *cmd)
                fault_cmds.append({'ns': 'r', 'cmd': cmd})
            t_fault = time.monotonic() - t0
        if stopped_at is None and t_fault is not None and now >= t_fault + WINDOW_S:
            stop.write_text('stop')
            stopped_at = now
        if stopped_at is not None and now >= stopped_at + POST_S:
            break
        ys, bs = tcp_sockets(box, 'y'), tcp_sockets(box, 'b')
        s = {'t': round(now, 2),
             'y_target': pick(ys, local=Y_ADDR, peer=f'{D_ADDR}:{chaos.PORT}'),
             'd_accepted': pick(bs, local=f'{D_ADDR}:{chaos.PORT}', peer=Y_ADDR),
             'y_outer': pick(ys, local=f'{Y_ADDR}:{chaos.PORT}', peer=A_ADDR),
             'auth_y': box.try_ctl('y', 'status').get('inbound_authority'),
             'auth_b': box.try_ctl('b', 'status').get('inbound_authority'),
             'res_y': box.sample('y'),
             'a_to_y': ping(box, 'a', Y_ADDR),
             'y_to_d': ping(box, 'y', D_ADDR)}
        samples.append(s)
        time.sleep(max(0, 1.0 - ((time.monotonic() - t0) - now)))
    record.update({'t_fault': t_fault, 't_stop': stopped_at, 'fault_cmds': fault_cmds,
                   'samples': samples, 'client': events(),
                   'client_stderr': client.stderr.read().decode()[-2000:] if client.poll() is not None else None,
                   'pids': pids, 'ports': ports, 'after': {n: box.sample(n) for n in nodes}})
    (out / f'{scenario.replace(":", "_")}.json').write_text(json.dumps(record, indent=1))
    for process in box.daemons.values():
        process.terminate()
    targets.terminate()


def main():
    if sys.argv[1] == '--inside':
        return inside(*sys.argv[2:6])
    bins, out, *scenarios = sys.argv[1:]
    os.makedirs(out, exist_ok=True)
    os.chmod(out, 0o755)
    for scenario in scenarios:
        work = tempfile.mkdtemp(prefix='wwp-s39-', dir=str(ROOT / 'target' / 'stage39-m1'))
        os.chmod(work, 0o755)
        started = time.time()
        rc = subprocess.run(['unshare', '-rmnp', '--fork', '--kill-child', '--', sys.executable,
                             '-B', __file__, '--inside', work, bins, scenario, out]).returncode
        print(json.dumps({'scenario': scenario, 'rc': rc, 'seconds': round(time.time() - started)}),
              flush=True)
        subprocess.run(['rm', '-rf', work])


if __name__ == '__main__':
    main()
