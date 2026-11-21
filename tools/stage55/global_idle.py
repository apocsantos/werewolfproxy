#!/usr/bin/env python3
"""Stage55 M1: reproduce idle denial at the production 1,024-slot global cap.

Disposable checkout-only plain-TCP lab. One 64-session Fang plus thirty
32-session Fangs fill the real cap; a separate empty Fang is a control.
All clients echo then wait at least 65 seconds. No runtime policy change,
shortened idle threshold, installed node, namespace change or keep-temp.
"""
import argparse
import collections
import json
import os
import pathlib
import resource
import selectors
import socket
import subprocess
import sys
import threading
import time

sys.dont_write_bytecode = True
ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/stage54'))
import idle_share as idle
from idle_share import support, require, until, open_fang
from peer_quota import attempt, exchange

GLOBAL = 1024
NOFILE = 4096


class Echo:
    """One epoll thread, buffered writes, no per-connection threads/timeouts."""
    def __init__(self):
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(128)
        self.listener.setblocking(False)
        self.address = f'127.0.0.1:{self.listener.getsockname()[1]}'
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.listener, selectors.EVENT_READ, None)
        self.lock = threading.Lock()
        self.connections = set()
        self.hits = 0
        self.errors = []
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def snapshot(self):
        with self.lock:
            return len(self.connections), self.hits, list(self.errors)

    def discard(self, conn):
        self.selector.unregister(conn)
        conn.close()
        with self.lock:
            self.connections.discard(conn)

    def run(self):
        try:
            while not self.stop.is_set():
                for key, events in self.selector.select(.05):
                    conn, state = key.fileobj, key.data
                    if state is None:
                        while True:
                            try:
                                client, _ = conn.accept()
                            except BlockingIOError:
                                break
                            client.setblocking(False)
                            self.selector.register(client, selectors.EVENT_READ,
                                                   {'buffer': bytearray(), 'counted': False})
                            with self.lock:
                                self.connections.add(client)
                        continue
                    try:
                        if events & selectors.EVENT_READ:
                            data = conn.recv(65536)
                            if not data:
                                self.discard(conn)
                                continue
                            if not state['counted']:
                                with self.lock:
                                    self.hits += 1
                                state['counted'] = True
                            state['buffer'].extend(data)
                        if events & selectors.EVENT_WRITE and state['buffer']:
                            sent = conn.send(state['buffer'])
                            del state['buffer'][:sent]
                        self.selector.modify(conn, selectors.EVENT_READ |
                                             (selectors.EVENT_WRITE if state['buffer'] else 0), state)
                    except BlockingIOError:
                        continue
                    except (ConnectionResetError, BrokenPipeError):
                        self.discard(conn)
        except Exception as exc:
            with self.lock:
                self.errors.append(f'{type(exc).__name__}: {exc}')
        finally:
            for key in list(self.selector.get_map().values()):
                self.selector.unregister(key.fileobj)
                key.fileobj.close()
            self.selector.close()
            with self.lock:
                self.connections.clear()

    def close(self):
        self.stop.set()
        self.thread.join(timeout=2)
        require(not self.thread.is_alive(), 'echo event loop failed to stop')


def scenario(lab, phase):
    target, connections = Echo(), []
    try:
        lab.boot()
        baseline = idle.fd_count(lab)
        require(lab.pack_literal('a', 'b')['exit'] == 0, 'service Pack setup')
        lab.ctl('a', 'service', 'add', 'global', 'wolf-b', target.address, 'tcp')
        groups = [{'fang': open_fang(lab, 'global', 'tcp-plain'), 'clients': [], 'slots': n}
                  for n in ([32] * 30 + [64])]
        unused = open_fang(lab, 'global', 'tcp-plain')

        def guard():
            lab.guard()
            require(all(p.poll() is None for p in lab.wolves), 'fixture daemon exited')
            require(not target.snapshot()[2], target.snapshot()[2])

        phase('filling', {'global_slots': GLOBAL, 'loaded_fangs': len(groups)})
        for group in groups:
            for _ in range(group['slots']):
                guard()
                idle.held(group['fang']['port'], group['clients'])
                connections.append(group['clients'][-1])
        require(len(connections) == GLOBAL, 'wrong production-cap workload')
        until(lab, lambda: target.snapshot()[:2] == (GLOBAL, GLOBAL), 'all target streams established')
        owner, control = groups[0], groups[-1]
        expected_fds = baseline + len(groups) + 1 + 2 * GLOBAL
        until(lab, lambda: idle.fd_count(lab) == expected_fds, 'stable saturated descriptor count')
        require(expected_fds < NOFILE, 'fixture would hit descriptor exhaustion')
        before = [attempt(group['fang']['port']) for group in (owner, control)]
        require(all(row['outcome'] == 'reset' for row in before), before)
        require(idle.states(connections) == ['open'] * GLOBAL, 'pre-idle client ended')

        phase('idle-wait', {'seconds': idle.IDLE, 'global_slots': GLOBAL})
        began = time.monotonic()
        while time.monotonic() - began < idle.IDLE:
            guard()
            time.sleep(min(1, max(0, idle.IDLE - (time.monotonic() - began))))
        elapsed = time.monotonic() - began
        require(elapsed >= idle.IDLE, 'shortened idle observation')
        require(idle.states(connections) == ['open'] * GLOBAL, 'idle clients ended without demand')
        phase('post-idle', {})
        denied = [attempt(owner['fang']['port']) for _ in range(3)]
        empty = [attempt(unused['port']) for _ in range(3)]
        require(all(row['outcome'] == 'reset' for row in denied + empty), denied + empty)
        require(idle.states(connections) == ['open'] * GLOBAL, 'global denial selected a victim')
        require(target.snapshot()[:2] == (GLOBAL, GLOBAL), 'denied newcomer reached target')

        positive = []
        require(attempt(control['fang']['port'], positive)['outcome'] == 'echo',
                'local 64-slot idle replacement failed at global capacity')
        connections.extend(positive)
        observed = idle.states(control['clients'])
        require(collections.Counter(observed) == {'reset': 1, 'open': 63}, observed)
        control['clients'] = [c for c, state in zip(control['clients'], observed) if state == 'open']
        others = sum((group['clients'] for group in groups[:-1]), [])
        require(idle.states(others) == ['open'] * 960, 'positive control evicted another Fang')
        until(lab, lambda: target.snapshot()[:2] == (GLOBAL, GLOBAL + 1), 'one replacement target')
        full_again = attempt(owner['fang']['port'])
        require(full_again['outcome'] == 'reset', 'positive control changed the global cap')

        # Release a different Fang's slot. Global availability alone must
        # recover this owner's admission without any extra old reset.
        groups[-2]['clients'].pop().close()
        until(lab, lambda: target.snapshot()[0] == GLOBAL - 1, 'released target stream drained')
        until(lab, lambda: idle.fd_count(lab) == expected_fds - 2, 'released local/target descriptors')
        recovery = []
        recovered = attempt(owner['fang']['port'], recovery)
        connections.extend(recovery)
        require(recovered['outcome'] == 'echo', recovered)
        survivors = sum((group['clients'] for group in groups), []) + positive + recovery
        require(len(survivors) == GLOBAL, 'wrong surviving client count')
        for conn in survivors:
            guard()
            exchange(conn)
        until(lab, lambda: target.snapshot()[:2] == (GLOBAL, GLOBAL + 2), 'recovered target count')
        cpu = support.idle_cpu(lab)
        phase('cleanup', {})
        for fang in lab.fang_list():
            lab.control('a', 'fang.close', {'fang_id': fang['id']})
        for conn in connections:
            conn.close()
        until(lab, lambda: target.snapshot()[0] == 0, 'all target streams drained')
        until(lab, lambda: idle.fd_count(lab) <= baseline, 'initiator descriptors returned')
        guard()
        return {'transport': 'tcp-plain', 'global_slots': GLOBAL, 'loaded_fangs': 31,
                'loads': [g['slots'] for g in groups], 'idle_seconds': round(elapsed, 3),
                'before_idle_probes': before, 'below_local_denials': denied,
                'empty_fang_denials': empty, 'old_open_after_denial': GLOBAL,
                'denied_payload_streams': 0, 'full_local_admitted': 1,
                'full_local_old_resets': 1, 'other_fang_clients_preserved': 960,
                'full_again': full_again, 'release_other_recovery': recovered,
                'surviving_echoes': len(survivors), 'idle_cpu_pct': cpu,
                'fds_before': baseline, 'fds_saturated': expected_fds,
                'fds_after': idle.fd_count(lab), 'target_streams_after': target.snapshot()[0],
                'target_payload_streams_total': target.snapshot()[1], 'daemons_alive': True,
                'nofile': NOFILE}
    finally:
        for conn in connections:
            conn.close()
        target.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--out', type=pathlib.Path, required=True)
    args = parser.parse_args()
    require(args.rounds > 0, 'invalid rounds')
    _, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    require(hard == resource.RLIM_INFINITY or hard >= NOFILE, '4096-descriptor lab limit unavailable')
    resource.setrlimit(resource.RLIMIT_NOFILE, (NOFILE, NOFILE))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open('x') as handle:
        handle.write('{}\n')
    os.umask(0o077)
    source = idle.inputs()
    source[str(pathlib.Path(__file__).resolve().relative_to(ROOT))] = idle.digest(pathlib.Path(__file__))
    document = {'status': 'RUNNING', 'source_inputs': source, 'rows': [],
                'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT,
                                                  text=True).strip()}
    for repetition in range(1, args.rounds + 1):
        idle.source_guard(source)
        lab = support.refresh.dial.DialLab()
        lab.WOLVES = ('a', 'b')
        row = {'round': repetition}
        def phase(label, detail):
            document['running'] = {**row, 'phase': label, **detail}
            idle.checkpoint(args.out, document)
            print(json.dumps(document['running']), flush=True)
        try:
            row.update(scenario(lab, phase), status='REPRODUCED')
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
    document['status'] = 'REPRODUCED'
    idle.checkpoint(args.out, document)
    return 0


if __name__ == '__main__':
    sys.exit(main())
