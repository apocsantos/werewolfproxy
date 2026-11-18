#!/usr/bin/env python3
"""Stage53 M3: membership changes, route demands and denial pressure.

Disposable Stage45/46 checkout lab, generated identities and dynamic
loopback ports. The 50 MiB log guard applies; only owned process groups
are stopped and all private fixture directories are removed. No live node.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time

sys.dont_write_bytecode = True
import paths
from paths import faults, refresh, require, until, active, open_fang, held
from peer_quota import attempt

ROOT = pathlib.Path(__file__).resolve().parents[2]
CASES = ('three-quic', 'three-tcp', 'three-mixed',
         'late-quic-quic', 'late-tcp-tcp', 'late-quic-tcp', 'late-tcp-quic',
         'relay-quic', 'relay-tcp', 'destination-quic', 'destination-tcp',
         'storm-quic', 'storm-tcp')


def pulse(lab, connections):
    lab.guard()
    for conn in connections:
        refresh.echo_held(conn)


def fill(lab, fang, count, connections):
    for _ in range(count):
        lab.guard()
        held(fang['port'], connections)


def close_fang(lab, fang, connections):
    lab.control('a', 'fang.close', {'fang_id': fang['id']})
    ends = []
    for conn in connections:
        try:
            data = conn.recv(1)
            ends.append('eof' if not data else 'data')
        except ConnectionResetError:
            ends.append('reset')
        conn.close()
    require(all(end == 'reset' for end in ends), f'Fang close did not reset sessions: {ends}')
    connections.clear()
    return len(ends)


def expect(lab, wolf, count):
    until(lab, lambda: active(lab, wolf) == count, f'{wolf}: {count} active, zero establishing')
    return lab.control(wolf, 'status')['inbound_authority']


def denied(lab, fang, count=2):
    rows = [attempt(fang['port']) for _ in range(count)]
    require(all(row['outcome'] == 'reset' for row in rows), rows)
    entry = next(row for row in lab.fang_list() if row['id'] == fang['id'])
    require(entry['last_failure']['class'] == 'dial', entry)
    return rows


def idle_cpu(lab):
    pid = lab.wolves[0].pid
    def ticks():
        fields = pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return int(fields[11]) + int(fields[12])
    before, began = ticks(), time.monotonic()
    while time.monotonic() - began < 2:
        lab.guard()
        time.sleep(.05)
    pct = 100 * (ticks() - before) / os.sysconf('SC_CLK_TCK') / (time.monotonic() - began)
    require(pct < 10, f'sustained initiator CPU while idle: {pct}')
    return round(pct, 2)


def setup(lab, targets):
    lab.boot()
    for wolf in ('b', 'c', 'd', 'e'):
        require(lab.pack_literal('a', wolf)['exit'] == 0, 'initiator Pack setup failed')
        require(lab.pack_literal(wolf, 'a')['exit'] == 0, 'receiver Pack setup failed')
        lab.ctl(wolf, 'target', 'allow', 'wolf-a', targets[wolf].address)
        lab.ctl('a', 'service', 'add', 'echo-' + wolf, 'wolf-' + wolf,
                targets[wolf].address, 'tcp')


def three(lab, transports, targets, conns, control):
    fangs = [open_fang(lab, 'echo-b', t) for t in transports]
    result = {'shares': [22, 21, 21]}
    for fang, limit, sockets in zip(fangs, result['shares'], conns):
        fill(lab, fang, limit, sockets)
        denied(lab, fang)
    result['full_authority'] = expect(lab, 'b', 64)
    pulse(lab, sum(conns, []) + control)
    result['idle_cpu_pct'] = idle_cpu(lab)
    pulse(lab, sum(conns, []) + control)
    result['third_close_resets'] = close_fang(lab, fangs[2], conns[2])
    result['after_third_close'] = expect(lab, 'b', 43)
    for fang, count, sockets in zip(fangs[:2], (10, 11), conns[:2]):
        fill(lab, fang, count, sockets)
        denied(lab, fang)
    result['two_shares'] = [len(sockets) for sockets in conns[:2]]
    result['two_full_authority'] = expect(lab, 'b', 64)
    result['second_close_resets'] = close_fang(lab, fangs[1], conns[1])
    expect(lab, 'b', 32)
    fill(lab, fangs[0], 32, conns[0])
    result['one_share'] = len(conns[0])
    result['one_full_authority'] = expect(lab, 'b', 64)
    pulse(lab, conns[0] + control)
    return result


def late(lab, transports, targets, conns, control):
    first = open_fang(lab, 'echo-b', transports[0])
    fill(lab, first, 64, conns[0])
    expect(lab, 'b', 64)
    second = open_fang(lab, 'echo-b', transports[1])
    result = {'late_denials': denied(lab, second, 3)}
    pulse(lab, conns[0] + control)
    result['grandfathered_authority'] = expect(lab, 'b', 64)
    result['idle_cpu_pct'] = idle_cpu(lab)
    pulse(lab, conns[0] + control)
    result['grandfathered_echoes'] = len(conns[0])
    conns[0].pop().close()
    expect(lab, 'b', 63)
    result['old_above_share_denials'] = denied(lab, first)
    fill(lab, second, 1, conns[1])
    result['released_slot_admitted_late_fang'] = expect(lab, 'b', 64)
    result['old_close_resets'] = close_fang(lab, first, conns[0])
    expect(lab, 'b', 1)
    fill(lab, second, 63, conns[1])
    result['remaining_share'] = len(conns[1])
    result['remaining_full_authority'] = expect(lab, 'b', 64)
    pulse(lab, conns[1] + control)
    return result


def routed(lab, mode, hop, targets, conns, control, reverse):
    if mode == 'relay':
        for dest in ('b', 'c'):
            lab.ctl('a', 'route', 'set', 'wolf-' + dest, '--via', 'wolf-d', '--hop', hop)
            lab.ctl('d', 'target', 'allow', 'wolf-a', lab.address(dest + '_tcp'))
        order = ('c', 'b') if reverse else ('b', 'c')
        fangs = [open_fang(lab, 'echo-' + dest, 'tcp') for dest in order]
    else:
        # Routes are captured per activation. Updating the route config is
        # for later opens; it does not rewrite an existing Fang's path.
        order = ('d', 'c') if reverse else ('c', 'd')
        fangs = []
        for relay in order:
            lab.ctl('a', 'route', 'set', 'wolf-b', '--via', 'wolf-' + relay, '--hop', hop)
            lab.ctl(relay, 'target', 'allow', 'wolf-a', lab.address('b_tcp'))
            fangs.append(open_fang(lab, 'echo-b', 'tcp'))
        shown = {row['id']: row['transport'] for row in lab.fang_list()}
        for fang, relay in zip(fangs, order):
            require(f'via wolf-{relay}' in shown[fang['id']], shown)
    result = {'order': list(order), 'captured_fangs': lab.fang_list()}
    fill(lab, fangs[0], 32, conns[0])
    result['first_share_denials'] = denied(lab, fangs[0], 3)
    fill(lab, fangs[1], 32, conns[1])
    denied(lab, fangs[1])
    if mode == 'relay':
        result['authority'] = {wolf: expect(lab, wolf, 64 if wolf == 'd' else 32)
                               for wolf in ('b', 'c', 'd')}
    else:
        result['authority'] = {wolf: expect(lab, wolf, 64 if wolf == 'b' else 32)
                               for wolf in ('b', 'c', 'd')}
    pulse(lab, sum(conns, []) + control)
    result['idle_cpu_pct'] = idle_cpu(lab)
    pulse(lab, sum(conns, []) + control)
    result['sibling_close_resets'] = close_fang(lab, fangs[1], conns[1])
    for wolf in ('b', 'c', 'd'):
        remaining = ({order[0]: 32, 'd': 32} if mode == 'relay'
                     else {'b': 32, order[0]: 32})
        expect(lab, wolf, remaining.get(wolf, 0))
    fill(lab, fangs[0], 32, conns[0])
    result['remaining_share'] = len(conns[0])
    remaining = ({order[0]: 64, 'd': 64} if mode == 'relay'
                 else {'b': 64, order[0]: 64})
    result['after_close_authority'] = {wolf: expect(lab, wolf, remaining.get(wolf, 0))
                                      for wolf in ('b', 'c', 'd')}
    pulse(lab, conns[0] + control)
    return result


def storm(lab, transport, targets, conns, control):
    first = open_fang(lab, 'echo-b', transport)
    open_fang(lab, 'echo-b', transport)  # Reserve an unused sibling share.
    fill(lab, first, 32, conns[0])
    expect(lab, 'b', 32)
    before_hits = targets['b'].hits
    before_lines = len(lab.a_log().splitlines())
    before_health = next(r for r in lab.fang_list() if r['id'] == first['id'])['sessions_failed']
    began = time.monotonic()
    # 32 held + 16 workers stays below the existing 64-slot Fang ceiling.
    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
        clients = list(pool.map(attempt, [first['port']] * 600))
    elapsed = time.monotonic() - began
    require(all(r['outcome'] == 'reset' for r in clients), clients)
    row = next(r for r in lab.fang_list() if r['id'] == first['id'])
    require(row['sessions_failed'] - before_health == 600 and
            row['last_failure']['class'] == 'dial', row)
    require(targets['b'].hits == before_hits, 'denied flood reached target')
    lines = len(lab.a_log().splitlines()) - before_lines
    require(lines <= 2 + int(elapsed / 10), f'unbounded denial logging: {lines}')
    pulse(lab, conns[0] + control)
    return {'clients': clients, 'concurrency': 16, 'held': len(conns[0]),
            'failure_count_delta': row['sessions_failed'] - before_health,
            'log_lines': lines, 'elapsed_s': round(elapsed, 3),
            'target_payload_stream_delta': targets['b'].hits - before_hits,
            'idle_cpu_pct': idle_cpu(lab)}


def scenario(lab, name, repetition):
    targets, conns, control = {}, [[], [], []], []
    try:
        targets = {wolf: faults.ControlledEcho() for wolf in ('b', 'c', 'd', 'e')}
        setup(lab, targets)
        baseline_fds = len(os.listdir(f'/proc/{lab.wolves[0].pid}/fd'))
        control_fang = open_fang(lab, 'echo-e', 'tcp')
        held(control_fang['port'], control)
        if name.startswith('three-'):
            suffix = name.split('-')[1]
            transports = ['quic', 'tcp', 'quic'] if suffix == 'mixed' else [suffix] * 3
            result = three(lab, transports, targets, conns, control)
        elif name.startswith('late-'):
            result = late(lab, name.split('-')[1:], targets, conns, control)
        elif name.startswith('storm-'):
            result = storm(lab, name.split('-')[1], targets, conns, control)
        else:
            result = routed(lab, *name.split('-'), targets, conns, control, repetition % 2 == 0)
        pulse(lab, control)
        result['independent_peer_held_echo'] = True
        result['independent_peer_new_clients'] = [attempt(control_fang['port']) for _ in range(3)]
        require(all(r['outcome'] == 'echo' for r in result['independent_peer_new_clients']),
                'independent peer could not admit new clients')
        for fang in lab.fang_list():
            lab.control('a', 'fang.close', {'fang_id': fang['id']})
        for conn in sum(conns, []) + control:
            conn.close()
        for sockets in conns:
            sockets.clear()
        control.clear()
        result['final_authority'] = {wolf: expect(lab, wolf, 0) for wolf in ('b','c','d','e')}
        until(lab, lambda: len(os.listdir(f'/proc/{lab.wolves[0].pid}/fd')) <= baseline_fds,
              'initiator descriptors returned after all Fangs closed')
        result['fds_before'] = baseline_fds
        result['fds_after_close'] = len(os.listdir(f'/proc/{lab.wolves[0].pid}/fd'))
        result['daemons_alive'] = all(p.poll() is None for p in lab.wolves)
        require(result['daemons_alive'], 'fixture daemon exited')
        return result
    finally:
        for conn in sum(conns, []) + control:
            conn.close()
        for target in targets.values():
            target.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('cases', nargs='*')
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--out', type=pathlib.Path, required=True)
    args = parser.parse_args()
    cases = args.cases or CASES
    require(args.rounds > 0 and all(c in CASES for c in cases), 'invalid rounds/cases')
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open('x') as output:
        output.write('[]\n')
    os.umask(0o077)
    rows = []
    for repetition in range(1, args.rounds + 1):
        for name in cases:
            lab = refresh.dial.DialLab()
            lab.WOLVES = ('a', 'b', 'c', 'd', 'e')
            row = {'case': name, 'round': repetition,
                   'commit': subprocess.check_output(['git','rev-parse','HEAD'], cwd=ROOT,
                                                     text=True).strip(),
                   'tool_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()}
            try:
                row.update(scenario(lab, name, repetition), status='PASS')
            except Exception as exc:
                row.update(status='FAIL', error=f'{type(exc).__name__}: {exc}',
                           failure_logs={p.name:p.read_text(errors='replace')[-2000:]
                                         for p in lab.base.glob('*.log')})
            finally:
                row['daemon_sha256'] = getattr(lab, 'binary_hash', None)
                try:
                    lab.cleanup()
                    require(not lab.base.exists() and all(p.poll() is not None for p in lab.processes),
                            'private fixture directory/process remains')
                    row['cleanup'] = 'PASS'
                except Exception as exc:
                    row.update(status='FAIL', cleanup_error=str(exc))
            rows.append(row)
            args.out.write_text(json.dumps(rows, indent=2) + '\n')
            print(json.dumps(row), flush=True)
    return 0 if all(r['status'] == 'PASS' for r in rows) else 1


if __name__ == '__main__':
    sys.exit(main())
