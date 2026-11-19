#!/usr/bin/env python3
"""Stage54 M2: idle primary reclamation, active fallback and commitment.

Disposable five-node Stage45/46 lab; 65 s real idle intervals. Direct and
routed QUIC/TCP providers, generated identities, dynamic loopback ports,
50 MiB log guard, owned-process cleanup. No installed node or keep-temp.
"""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time

sys.dont_write_bytecode = True
import idle_share as idle
from idle_share import support, require, held, open_fang
from paths import configure, until, active

PATHS = ('quic', 'tcp', 'routed-quic', 'routed-tcp')


def scenario(lab, path, phase):
    targets, primary, fallback, controls, new = {}, [], [], [], []
    try:
        targets = {wolf: support.faults.ControlledEcho() for wolf in ('b', 'c', 'e')}
        lab.boot()
        baseline = idle.fd_count(lab)
        configure(lab, targets, path)
        for wolf, peer in (('a', 'e'), ('e', 'a')):
            require(lab.pack_literal(wolf, peer)['exit'] == 0, 'independent Pack setup failed')
        lab.ctl('e', 'target', 'allow', 'wolf-a', targets['e'].address)
        lab.ctl('a', 'service', 'add', 'independent', 'wolf-e', targets['e'].address, 'tcp')
        transport = 'quic' if path == 'quic' else 'tcp'
        open_fang(lab, 'unused', transport)
        fang = open_fang(lab, 'quota', transport)
        independent = open_fang(lab, 'independent', 'tcp')
        held(independent['port'], controls)
        for _ in range(32):
            lab.guard()
            held(fang['port'], primary)
        support.expect(lab, 'b', 32)
        if path.startswith('routed'):
            support.expect(lab, 'd', 32)
        # Recent primary payload is protected: capacity denial uses fallback.
        held(fang['port'], fallback)
        require(targets['b'].hits == 32 and targets['c'].hits == 1,
                'active primary saturation did not use the next provider')
        support.pulse(lab, primary + fallback + controls)
        phase('idle-wait', {'seconds': idle.IDLE, 'primary_held': 32, 'fallback_held': 1})
        began = time.monotonic()
        while time.monotonic() - began < idle.IDLE:
            lab.guard()
            require(all(p.poll() is None for p in lab.wolves), 'fixture daemon exited')
            support.pulse(lab, fallback + controls)
            time.sleep(min(1, max(0, idle.IDLE - (time.monotonic() - began))))
        elapsed = time.monotonic() - began
        require(elapsed >= idle.IDLE, 'shortened idle observation')
        require(idle.states(primary) == ['open'] * 32, 'primary sessions ended before demand')
        before = {wolf: target.hits for wolf, target in targets.items()}
        for _ in range(3):
            held(fang['port'], new)
        require(targets['b'].hits - before['b'] == 3 and targets['c'].hits == before['c'],
                'eligible idle primary was passed over for fallback')
        observed = idle.states(primary)
        require(observed.count('reset') == 3 and observed.count('open') == 29,
                f'not exactly one old idle primary reset per newcomer: {observed}')
        survivors = [conn for conn, state in zip(primary, observed) if state == 'open']
        support.pulse(lab, survivors + new + fallback + controls)
        support.expect(lab, 'b', 32)
        if path.startswith('routed'):
            support.expect(lab, 'd', 32)
        # All live primary sessions are committed. Their target fault must
        # reset them without replay through C; existing fallback stays live.
        fallback_before_fault = targets['c'].hits
        targets['b'].abort.set()
        ends = []
        for conn in survivors + new:
            try:
                data = conn.recv(1)
                ends.append('eof' if not data else 'data')
            except ConnectionResetError:
                ends.append('reset')
        require(ends == ['reset'] * 32, f'committed primary outcomes: {ends}')
        support.expect(lab, 'b', 0)
        if path.startswith('routed'):
            support.expect(lab, 'd', 0)
        until(lab, lambda: not targets['b'].connections, 'aborted primary target drained')
        require(targets['c'].hits == fallback_before_fault, 'post-ACK payload replayed to fallback')
        support.pulse(lab, fallback + controls)
        targets['b'].abort.clear()
        recovered = []
        held(fang['port'], recovered)
        new.extend(recovered)
        require(targets['b'].hits == before['b'] + 4 and targets['c'].hits == fallback_before_fault,
                'primary did not recover without reopen')
        independent_new = [support.attempt(independent['port']) for _ in range(3)]
        require(all(row['outcome'] == 'echo' for row in independent_new), 'independent new client failed')
        for entry in lab.fang_list():
            lab.control('a', 'fang.close', {'fang_id': entry['id']})
        for conn in primary + fallback + controls + new:
            conn.close()
        final = {wolf: support.expect(lab, wolf, 0) for wolf in ('b', 'c', 'd', 'e')}
        until(lab, lambda: idle.fd_count(lab) <= baseline, 'initiator descriptors drained')
        for target in targets.values():
            until(lab, lambda target=target: not target.connections, 'target connections drained')
        require(all(p.poll() is None for p in lab.wolves), 'fixture daemon exited')
        return {'idle_seconds': round(elapsed, 3), 'active_primary_fallback': 1,
                'idle_primary_admissions': 3, 'idle_primary_old_resets': observed.count('reset'),
                'committed_primary_resets': len(ends), 'fallback_payload_replays': 0,
                'fallback_held_survived': True, 'primary_recovered_without_reopen': True,
                'independent_new_clients': independent_new, 'final_authority': final,
                'fds_before': baseline, 'fds_after': idle.fd_count(lab), 'daemons_alive': True}
    finally:
        for conn in primary + fallback + controls + new:
            conn.close()
        for target in targets.values():
            target.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('paths', nargs='*')
    parser.add_argument('--out', type=pathlib.Path, required=True)
    args = parser.parse_args()
    paths = args.paths or PATHS
    require(all(path in PATHS for path in paths), 'invalid provider path')
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open('x') as output:
        output.write('{}\n')
    os.umask(0o077)
    source = idle.inputs()
    document = {'status': 'RUNNING', 'source_inputs': source, 'rows': [],
                'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=idle.ROOT, text=True).strip(),
                'tool_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()}
    for path in paths:
        idle.source_guard(source)
        lab = support.refresh.dial.DialLab()
        lab.WOLVES = ('a', 'b', 'c', 'd', 'e')
        row = {'path': path}
        def phase(name, detail):
            document['running'] = {'path': path, 'phase': name, **detail}
            idle.checkpoint(args.out, document)
            print(json.dumps(document['running']), flush=True)
        try:
            row.update(scenario(lab, path, phase), status='PASS')
            idle.source_guard(source)
        except Exception as exc:
            row.update(status='FAIL', error=f'{type(exc).__name__}: {exc}',
                       failure_logs={p.name: p.read_text(errors='replace')[-2000:]
                                     for p in lab.base.glob('*.log')})
        finally:
            row['daemon_sha256'] = getattr(lab, 'binary_hash', None)
            try:
                lab.cleanup()
                require(not lab.base.exists() and all(p.poll() is not None for p in lab.processes),
                        'private fixture directory or owned process remains')
                row['cleanup'] = 'PASS'
            except Exception as exc:
                row.update(status='FAIL', cleanup_error=str(exc))
        document['rows'].append(row)
        document.pop('running', None)
        idle.checkpoint(args.out, document)
        print(json.dumps(row), flush=True)
        if row['status'] == 'FAIL':
            document['status'] = 'FAIL'
            idle.checkpoint(args.out, document)
            return 1
    document['status'] = 'PASS'
    idle.checkpoint(args.out, document)
    return 0


if __name__ == '__main__':
    sys.exit(main())
