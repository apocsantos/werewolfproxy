#!/usr/bin/env python3
"""Stage53 local-denial failover/commitment and activation rollback controls.

Disposable checkout-only Stage45/46 lab and Stage52 counted/reset targets.
No resolver injection, namespace, installed node or retained identity.
"""
import argparse
import hashlib
import json
import os
import pathlib
import socket
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / 'tools/stage52'))
import faults  # noqa: E402
from faults import refresh, require  # noqa: E402


def until(lab, predicate, description):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        lab.guard()
        if predicate():
            return
        time.sleep(.05)
    raise RuntimeError(f'timeout: {description}')


def open_fang(lab, service, transport):
    lab.reserve('extra')
    port = lab.ports['extra']
    lab.release('extra')
    opened = lab.ctl_any('a', 'service', 'connect', service, '--local-port', port,
                         '--transport', transport)
    require(opened['exit'] == 0, opened)
    return {'port': port, 'id': opened['body']['result']['fang_id']}


def held(port, conns):
    conn = socket.create_connection(('127.0.0.1', port), timeout=10)
    conns.append(conn)
    conn.settimeout(10)
    refresh.echo_held(conn)


def active(lab, wolf):
    s = lab.control(wolf, 'status')['inbound_authority']
    return s['active'] if s['establishing'] == 0 else -1


def raw_open(lab, port, transport):
    with socket.socket(socket.AF_UNIX) as control:
        control.settimeout(10)
        control.connect(str(lab.base / 'a.sock'))
        control.sendall((json.dumps({'id': 'stage53-error', 'cmd': 'service.connect',
                                    'args': {'name': 'one', 'local': f'127.0.0.1:{port}',
                                             'transport': transport}}) + '\n').encode())
        with control.makefile('rb') as response:
            result = json.loads(response.readline(1024 * 1024))
        require(result['id'] == 'stage53-error', result)
        return result


def configure(lab, targets, path):
    for wolf in ('b', 'c'):
        lab.pack_literal('a', wolf)
        lab.pack_literal(wolf, 'a')
        lab.ctl(wolf, 'target', 'allow', 'wolf-a', targets[wolf].address)
    if path.startswith('routed'):
        lab.pack_literal('a', 'd')
        lab.pack_literal('d', 'a')
        lab.ctl('a', 'route', 'set', 'wolf-b', '--via', 'wolf-d', '--hop', path.split('-')[1])
        lab.ctl('d', 'target', 'allow', 'wolf-a', lab.address('b_tcp'))
    lab.ctl('a', 'service', 'add', 'quota', 'wolf-b', targets['b'].address, 'tcp')
    lab.ctl('a', 'service', 'provider', 'add', 'quota', 'wolf-c', targets['c'].address)
    # Its unused membership reserves half of B's share for a sibling.
    lab.ctl('a', 'service', 'add', 'unused', 'wolf-b', targets['b'].address, 'tcp')


def scenario(lab, path):
    conns, fallbacks = [], []
    targets = {}
    try:
        lab.boot()
        targets = {wolf: faults.ControlledEcho() for wolf in ('b', 'c')}
        configure(lab, targets, path)
        transport = 'tcp' if path == 'tcp' or path.startswith('routed') else 'quic'
        unused = open_fang(lab, 'unused', transport)
        fang = open_fang(lab, 'quota', transport)
        for _ in range(32):
            lab.guard()
            held(fang['port'], conns)
        require(targets['b'].hits == 32 and targets['c'].hits == 0,
                'first 32 sessions did not use primary')
        until(lab, lambda: active(lab, 'b') == 32, '32 primary sessions')
        if path.startswith('routed'):
            until(lab, lambda: active(lab, 'd') == 32, '32 relay sessions')
        held(fang['port'], fallbacks)
        require(targets['b'].hits == 32 and targets['c'].hits == 1,
                '33rd session did not use fallback on local capacity denial')
        for conn in conns + fallbacks:
            refresh.echo_held(conn)
        result = {'primary_payload_streams_at_share': 32, 'local_denial_fallback': 1}
        # A committed primary reset must not turn into a fallback replay.
        before = targets['c'].hits
        targets['b'].abort.set()
        ends = []
        for conn in conns:
            try:
                data = conn.recv(1)
                ends.append('eof' if not data else 'data')
            except ConnectionResetError:
                ends.append('reset')
        require(ends == ['reset'] * 32, f'committed clients: {ends}')
        time.sleep(.1)
        require(targets['c'].hits == before, 'committed sessions replayed to fallback')
        result['committed_resets'] = len(ends)
        result['fallback_payload_replays'] = targets['c'].hits - before
        for conn in fallbacks:
            refresh.echo_held(conn)
        until(lab, lambda: active(lab, 'b') == 0, 'primary authority cleanup')
        if path.startswith('routed'):
            until(lab, lambda: active(lab, 'd') == 0, 'relay authority cleanup')
        # All reset target handlers must exit before clearing the fault.
        until(lab, lambda: not targets['b'].connections, 'reset target handler cleanup')
        targets['b'].abort.clear()
        held(fang['port'], conns)
        require(targets['b'].hits == 33 and targets['c'].hits == 1,
                'primary capacity failed to recover')
        result['primary_recovered'] = True
        result['health'] = lab.fang_list()
        require(all(p.poll() is None for p in lab.wolves), 'fixture daemon exited')
        result['daemons_alive'] = True
        lab.control('a', 'fang.close', {'fang_id': unused['id']})
        return result
    finally:
        for conn in conns + fallbacks:
            conn.close()
        for target in targets.values():
            target.close()


def activation(lab):
    target = faults.ControlledEcho()
    conns = []
    try:
        lab.boot()
        for wolf, peer in (('a', 'b'), ('b', 'a')):
            lab.pack_literal(wolf, peer)
        lab.ctl('b', 'target', 'allow', 'wolf-a', target.address)
        lab.ctl('a', 'service', 'add', 'one', 'wolf-b', target.address, 'tcp')
        fangs = [open_fang(lab, 'one', 'tcp') for _ in range(64)]
        lab.reserve('rejected')
        port = lab.ports['rejected']
        lab.release('rejected')
        rejected = raw_open(lab, port, 'quic')
        require(not rejected['ok'] and
                rejected['error']['code'] == 'FANG_PEER_CAPACITY', rejected)
        require(len(lab.fang_list()) == 64, 'failed activation left a record')
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', port))
        held(fangs[0]['port'], conns)
        require(refresh.dial.attempt(fangs[0]['port']) == 'reset', 'unused 63 shares were borrowed')
        for conn in conns:
            conn.close()
        conns.clear()
        until(lab, lambda: active(lab, 'b') == 0, 'single session cleanup')
        for fang in fangs[:-1]:
            lab.control('a', 'fang.close', {'fang_id': fang['id']})
        # Register, then fail listener bind: every provisional share rolls back.
        lab.reserve('occupied')
        failed = raw_open(lab, lab.ports['occupied'], 'tcp')
        require(not failed['ok'] and
                failed['error']['code'] == 'FANG_LISTENER_FAILED', failed)
        lab.release('occupied')
        for _ in range(64):
            lab.guard()
            held(fangs[-1]['port'], conns)
        until(lab, lambda: active(lab, 'b') == 64, '64 slots after membership shrink/rollback')
        return {'registered': 64, 'sixty_fifth_error': rejected['error']['code'],
                'failed_open_left_port_free': True, 'unused_shares_not_borrowed': True,
                'bind_failure_rolled_back': True, 'after_close_63_echoes': len(conns)}
    finally:
        for conn in conns:
            conn.close()
        target.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('paths', nargs='*', help='default: all provider paths and activation')
    parser.add_argument('--out', type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open('x') as output:
        output.write('[]\n')
    os.umask(0o077)
    rows = []
    paths = args.paths or ('quic', 'tcp', 'routed-quic', 'routed-tcp', 'activation')
    require(all(p in ('quic', 'tcp', 'routed-quic', 'routed-tcp', 'activation') for p in paths),
            'unknown control path')
    for path in paths:
        lab = refresh.dial.DialLab()
        lab.WOLVES = ('a', 'b', 'c', 'd')
        row = {'path': path, 'commit': subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            'tool_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()}
        try:
            row.update(activation(lab) if path == 'activation' else scenario(lab, path), status='PASS')
        except Exception as exc:
            row.update(status='FAIL', error=f'{type(exc).__name__}: {exc}')
            row['failure_logs'] = {p.name: p.read_text(errors='replace')[-2000:]
                                   for p in lab.base.glob('*.log')}
        finally:
            row['daemon_sha256'] = getattr(lab, 'binary_hash', None)
            try:
                lab.cleanup()
                require(not lab.base.exists(), 'private fixture remains')
                row['cleanup'] = 'PASS'
            except Exception as exc:
                row.update(status='FAIL', cleanup_error=str(exc))
        rows.append(row)
        args.out.write_text(json.dumps(rows, indent=2) + '\n')
        print(json.dumps(row), flush=True)
    return 0 if all(row['status'] == 'PASS' for row in rows) else 1


if __name__ == '__main__':
    sys.exit(main())
