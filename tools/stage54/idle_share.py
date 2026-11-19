#!/usr/bin/env python3
"""Stage54 M1: measure idle admission at 64/32/21 peer-session shares.

Disposable checkout-only five-node lab. Expected smaller-share denial is
a baseline reproduction, not a fix. No installed node, namespace change,
retained identity or shortened idle threshold. Output must be new.
"""
import argparse
import collections
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
sys.path.insert(0, str(ROOT / 'tools/stage53'))
import membership as support
from paths import active, held, open_fang, require, until
from peer_quota import attempt

IDLE = 65


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inputs():
    tracked = subprocess.check_output(['git', 'ls-files'], cwd=ROOT, text=True).splitlines()
    names = [name for name in tracked if pathlib.Path(name).suffix in
             ('.rs', '.toml', '.lock', '.py', '.sh', '.c')]
    names.append(str(pathlib.Path(__file__).resolve().relative_to(ROOT)))
    return {name: digest(ROOT / name) for name in sorted(set(names))}


def source_guard(expected):
    require(all(digest(ROOT / name) == value for name, value in expected.items()),
            'source inputs changed during reproduction')


def checkpoint(path, document):
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(document, indent=2) + '\n')
    temporary.replace(path)


def states(connections):
    result = []
    for conn in connections:
        timeout = conn.gettimeout()
        conn.setblocking(False)
        try:
            data = conn.recv(1, socket.MSG_PEEK)
            result.append('eof' if not data else 'unexpected-data')
        except BlockingIOError:
            result.append('open')
        except ConnectionResetError:
            result.append('reset')
        finally:
            conn.settimeout(timeout)
    return result


def fd_count(lab):
    return len(list(pathlib.Path(f'/proc/{lab.wolves[0].pid}/fd').iterdir()))


def scenario(lab, transport, phase):
    targets, connections = {}, []
    try:
        targets = {wolf: support.faults.ControlledEcho() for wolf in ('b', 'c', 'd', 'e')}
        support.setup(lab, targets)
        baseline = fd_count(lab)
        # Distinct receiver keys isolate each budget while one wait tests
        # all three. D's second member gets 21, not the 22-slot remainder.
        groups = []
        controls = []
        for wolf, members, owner_index, share in (('b', 1, 0, 64),
                                                  ('c', 2, 0, 32),
                                                  ('d', 3, 1, 21)):
            fangs = [open_fang(lab, 'echo-' + wolf, transport) for _ in range(members)]
            owner = fangs[owner_index]
            idle = []
            for fang in fangs:
                if fang is not owner:
                    held(fang['port'], controls)
                    connections.append(controls[-1])
            for _ in range(share):
                lab.guard()
                held(owner['port'], idle)
                connections.append(idle[-1])
            group = {'receiver': wolf, 'members': members, 'share': share,
                     'owner': owner, 'idle': idle, 'siblings': members - 1}
            group['before_idle_probes'] = [attempt(owner['port']) for _ in range(3)]
            require(all(p['outcome'] == 'reset' for p in group['before_idle_probes']),
                    'admitted a newcomer before the idle threshold')
            group['before_authority'] = support.expect(lab, wolf, share + members - 1)
            groups.append(group)
        independent = open_fang(lab, 'echo-e', 'tcp')
        held(independent['port'], controls)
        connections.append(controls[-1])
        # No bytes touch any idle owner connection during this interval.
        # Only sibling/independent controls are refreshed, once per second.
        began = time.monotonic()
        phase('idle-wait', {'seconds': IDLE, 'shares': [64, 32, 21]})
        pulses = 0
        while time.monotonic() - began < IDLE:
            lab.guard()
            require(all(p.poll() is None for p in lab.wolves), 'fixture daemon exited')
            support.pulse(lab, controls)
            pulses += 1
            time.sleep(min(1, max(0, IDLE - (time.monotonic() - began))))
        elapsed = time.monotonic() - began
        require(elapsed >= IDLE, 'idle interval was shortened')
        rows = []
        for group in groups:
            wolf, share = group['receiver'], group['share']
            idle, owner = group['idle'], group['owner']
            require(states(idle) == ['open'] * share, 'idle sessions ended before newcomer')
            before_hits = targets[wolf].hits
            newcomer = []
            probes = [attempt(owner['port'], newcomer) for _ in range(1 if share == 64 else 3)]
            connections.extend(newcomer)
            observed = states(idle)
            if share == 64:
                require(probes[0]['outcome'] == 'echo', 'full-Fang idle eviction control failed')
                require(observed.count('reset') == 1 and observed.count('open') == 63,
                        f'full Fang did not reset exactly one old client: {observed}')
                require(targets[wolf].hits - before_hits == 1, 'new full-Fang client missed target')
                survivors = [conn for conn, state in zip(idle, observed) if state == 'open']
                support.pulse(lab, survivors + newcomer)
                recovered = None
            else:
                require(all(p['outcome'] == 'reset' for p in probes),
                        'smaller-share baseline denial did not reproduce')
                require(observed == ['open'] * share, 'smaller share evicted an old client')
                require(targets[wolf].hits == before_hits, 'denied clients reached target')
                health = next(row for row in lab.fang_list() if row['id'] == owner['id'])
                require(health['sessions_failed'] == 6 and
                        health['last_failure']['class'] == 'dial', health)
                # Explicitly releasing one slot must recover without reopen.
                # Probe remaining idle sessions only AFTER observing denial.
                idle[-1].close()
                support.expect(lab, wolf, share + group['siblings'] - 1)
                recovered = attempt(owner['port'], newcomer)
                connections.extend(newcomer)
                require(recovered['outcome'] == 'echo', 'release-one recovery failed')
                support.pulse(lab, idle[:-1] + newcomer)
            authority = support.expect(lab, wolf, share + group['siblings'])
            rows.append({'share': share, 'members': group['members'], 'receiver': wolf,
                         'before_idle_probes': group['before_idle_probes'],
                         'before_authority': group['before_authority'],
                         'idle_before_newcomer': share,
                         'after_idle_probes': probes,
                         'old_client_states': dict(collections.Counter(observed)),
                         'newcomer_payload_streams': 1 if share == 64 else 0,
                         'release_one_recovery': recovered,
                         'after_authority': authority})
        support.pulse(lab, controls)
        independent_new = [attempt(independent['port']) for _ in range(3)]
        require(all(row['outcome'] == 'echo' for row in independent_new),
                'independent peer new admission failed')
        for fang in lab.fang_list():
            lab.control('a', 'fang.close', {'fang_id': fang['id']})
        for conn in connections:
            conn.close()
        final = {wolf: support.expect(lab, wolf, 0) for wolf in ('b', 'c', 'd', 'e')}
        until(lab, lambda: fd_count(lab) <= baseline, 'initiator descriptors returned to baseline')
        for target in targets.values():
            until(lab, lambda target=target: not target.connections, 'target connections drained')
        require(all(p.poll() is None for p in lab.wolves), 'fixture daemon exited')
        return {'idle_seconds': round(elapsed, 3), 'control_echo_rounds': pulses,
                'groups': rows, 'sibling_and_independent_held_echo': True,
                'independent_new_clients': independent_new,
                'fds_before': baseline, 'fds_after': fd_count(lab),
                'final_authority': final, 'daemons_alive': True}
    finally:
        for conn in connections:
            conn.close()
        for target in targets.values():
            target.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('transports', nargs='*')
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--out', type=pathlib.Path, required=True)
    args = parser.parse_args()
    transports = args.transports or ('quic', 'tcp')
    require(args.rounds > 0 and all(t in ('quic', 'tcp') for t in transports), 'invalid matrix')
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open('x') as handle:
        handle.write('{}\n')
    os.umask(0o077)
    source = inputs()
    document = {'expectation': 'baseline-limitation', 'status': 'RUNNING',
                'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT,
                                                  text=True).strip(),
                'source_inputs': source, 'rows': []}
    for repetition in range(1, args.rounds + 1):
        for transport in transports:
            source_guard(source)
            lab = support.refresh.dial.DialLab()
            lab.WOLVES = ('a', 'b', 'c', 'd', 'e')
            row = {'round': repetition, 'transport': transport}
            def phase(name, detail):
                document['running'] = {**row, 'phase': name, **detail}
                checkpoint(args.out, document)
                print(json.dumps(document['running']), flush=True)
            try:
                row.update(scenario(lab, transport, phase), status='REPRODUCED')
                source_guard(source)
            except Exception as exc:
                row.update(status='FAIL', error=f'{type(exc).__name__}: {exc}',
                           failure_logs={p.name: p.read_text(errors='replace')[-2000:]
                                         for p in lab.base.glob('*.log')})
            finally:
                row['daemon_sha256'] = getattr(lab, 'binary_hash', None)
                try:
                    lab.cleanup()
                    require(not lab.base.exists() and
                            all(p.poll() is not None for p in lab.processes),
                            'private fixture directory or owned process remains')
                    row['cleanup'] = 'PASS'
                except Exception as exc:
                    row.update(status='FAIL', cleanup_error=str(exc))
            document['rows'].append(row)
            document.pop('running', None)
            checkpoint(args.out, document)
            print(json.dumps(row), flush=True)
            if row['status'] == 'FAIL':
                document['status'] = 'FAIL'
                checkpoint(args.out, document)
                return 1
    document['status'] = 'REPRODUCED'
    checkpoint(args.out, document)
    return 0


if __name__ == '__main__':
    sys.exit(main())
