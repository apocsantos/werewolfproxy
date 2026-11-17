#!/usr/bin/env python3
"""Stage53: reproduce two Fangs competing for one receiver's peer quota.

Checkout-only Stage42/acceptance lab, three generated identities, dynamic
loopback ports and the existing 50 MiB log guard. No installed nodes,
namespace/network changes or retained private identities. Expected denial
is a successful reproduction, not a runtime fix. Unexpected outcomes or
cleanup errors return nonzero. Output must be a new file.
"""
import argparse
import hashlib
import json
import os
import pathlib
import socket
import socketserver
import subprocess
import sys
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / 'tools/stage42'))
from fang_flood import FloodLab, require, write_json  # noqa: E402

PAIRS = ('quic-quic', 'tcp-tcp', 'quic-tcp', 'tcp-quic')
PAYLOAD = b'stage53-peer-quota-echo'


def exchange(conn):
    conn.sendall(PAYLOAD)
    got = b''
    while len(got) < len(PAYLOAD):
        data = conn.recv(len(PAYLOAD) - len(got))
        require(data, 'EOF before echo')
        got += data
    require(got == PAYLOAD, 'echo payload differs')


def attempt(port, held=None):
    started = time.monotonic()
    conn = None
    try:
        conn = socket.create_connection(('127.0.0.1', port), timeout=5)
        conn.settimeout(5)
        exchange(conn)
        if held is not None:
            held.append(conn)
            conn = None
        outcome = 'echo'
    except (ConnectionResetError, BrokenPipeError):
        outcome = 'reset'
    except (socket.timeout, TimeoutError):
        outcome = 'timeout'
    except RuntimeError as exc:
        if str(exc) != 'EOF before echo':
            raise
        outcome = 'eof'
    finally:
        if conn is not None:
            conn.close()
    return {'outcome': outcome, 'seconds': round(time.monotonic() - started, 4)}


class EchoHandler(socketserver.BaseRequestHandler):
    def handle(self):
        counted = False
        self.request.settimeout(15)
        try:
            while data := self.request.recv(65536):
                if not counted:
                    with self.server.lock:
                        self.server.hits += 1
                    counted = True
                self.request.sendall(data)
        except OSError:
            pass


class Echo(socketserver.ThreadingTCPServer):
    daemon_threads = True

    def __init__(self):
        super().__init__(('127.0.0.1', 0), EchoHandler)
        self.lock = threading.Lock()
        self.hits = 0
        self.thread = threading.Thread(target=self.serve_forever,
                                       kwargs={'poll_interval': .05}, daemon=True)
        self.thread.start()

    def close(self):
        self.shutdown()
        self.server_close()
        self.thread.join(timeout=2)
        require(not self.thread.is_alive(), 'echo server failed to stop')


class QuotaLab(FloodLab):
    WOLVES = ('a', 'b', 'c')

    def setup_pair(self, left, right, targets):
        self.prepare_binaries()
        self.reserve_wolf_ports()
        for name in ('left', 'right', 'independent'):
            self.reserve(name)
        self.start_wolves()
        identities = {}
        for wolf in self.WOLVES:
            self.control(wolf, 'pelt.init')
            pelt = json.loads((self.base / wolf / 'pelt.json').read_text())
            identities[wolf] = {k: pelt[k] for k in ('fingerprint', 'public_key_b64')}
        for wolf, process in zip(self.WOLVES, self.wolves):
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'),
                          f'{wolf} TCP', process)
        self.stop_wolves()

        def peer(wolf):
            return dict(name='wolf-' + wolf, trust='Packmate', **identities[wolf],
                        address='quic://' + self.address(wolf + '_quic'))

        write_json(self.base / 'a/pack.json', [peer('b'), peer('c')])
        policy = {'mode': 'deny-by-default', 'peers': {}}
        for wolf in ('b', 'c'):
            write_json(self.base / wolf / 'pack.json', [peer('a')])
            write_json(self.base / wolf / 'target_policy.json',
                       {'mode': 'deny-by-default', 'peers': {}})
            policy['peers'][identities[wolf]['fingerprint']] = {'targets': [
                {'address': '127.0.0.1', 'port': target.server_address[1]}
                for target in targets.values()]}
        write_json(self.base / 'a/target_policy.json', policy)
        self.start_wolves()
        for name, wolf, transport in (('left', 'b', left), ('right', 'b', right),
                                      ('independent', 'c', 'tcp')):
            self.release(name)
            self.control(wolf, 'fang.profile.add', dict(
                name=name, peer='wolf-a', local=self.address(name),
                remote=f'127.0.0.1:{targets[name].server_address[1]}', transport=transport))
            self.control(wolf, 'fang.open_profile', {'name': name})
        self.expect_sessions(0)

    def authority(self):
        return self.control('a', 'status')['inbound_authority']

    def expect_sessions(self, active):
        def check():
            self.guard()
            summary = self.authority()
            require(summary['healthy'] and not summary['locked'], summary)
            require(summary['active'] == active and summary['establishing'] == 0, summary)
            return summary
        return self.wait_for(check, f'{active} receiver sessions')

    def health(self):
        return self.control('b', 'fang.list')


def pulse(lab, conns):
    lab.guard()
    for conn in conns:
        exchange(conn)


def phase(lab, owner, other, targets, active, conns, control):
    result = {'owner': owner, 'contender': other}
    conns.clear()
    filled = []
    for _ in range(64):
        lab.guard()
        filled.append(attempt(lab.ports[owner], conns))
    require(all(row['outcome'] == 'echo' for row in filled), f'fill failed: {filled}')
    result['fill'] = filled
    result['saturated_authority'] = lab.expect_sessions(65)
    before = targets[other].hits
    result['contender_immediate'] = [attempt(lab.ports[other]) for _ in range(3)]
    result['independent_new_clients'] = [attempt(lab.ports['independent']) for _ in range(3)]
    require(all(row['outcome'] == 'echo' for row in result['independent_new_clients']),
            'independent peer could not open new sessions during saturation')
    started = time.monotonic()
    pulses = 0
    while time.monotonic() - started < active:
        pulse(lab, conns + control)
        pulses += 1
        lab.wait(min(.25, max(0, active - (time.monotonic() - started))))
    result['active_hold_seconds'] = round(time.monotonic() - started, 3)
    result['active_echo_rounds'] = pulses
    result['contender_after_active'] = [attempt(lab.ports[other]) for _ in range(3)]
    for key in ('contender_immediate', 'contender_after_active'):
        require(all(row['outcome'] == 'reset' for row in result[key]),
                f'expected quota resets: {result[key]}')
    result['denied_target_payload_streams'] = targets[other].hits - before
    require(result['denied_target_payload_streams'] == 0, 'denied contender reached target')
    result['health_while_denied'] = lab.health()
    result['still_saturated_authority'] = lab.expect_sessions(65)
    pulse(lab, conns + control)
    result['existing_sessions_echo'] = len(conns) + len(control)

    # Exactly one owner session releases: the other Fang immediately takes
    # that peer slot without reopen. Keep it open to prove capacity is shared.
    conns.pop().close()
    result['one_released_authority'] = lab.expect_sessions(64)
    result['contender_after_one_release'] = attempt(lab.ports[other], conns)
    require(result['contender_after_one_release']['outcome'] == 'echo', result)
    result['refilled_authority'] = lab.expect_sessions(65)
    result['owner_after_refill'] = attempt(lab.ports[owner])
    require(result['owner_after_refill']['outcome'] == 'reset', result)
    pulse(lab, conns + control)
    for conn in conns:
        conn.close()
    conns.clear()
    result['released_authority'] = lab.expect_sessions(1)
    result['both_after_release'] = {name: attempt(lab.ports[name])
                                    for name in ('left', 'right')}
    require(all(row['outcome'] == 'echo' for row in result['both_after_release'].values()),
            'Fang did not recover without reopen')
    lab.expect_sessions(1)
    return result


def phase_shares(lab, owner, other, targets, active, conns, control):
    result = {'owner': owner, 'sibling': other}
    conns.clear()
    result['owner_fill'] = [attempt(lab.ports[owner], conns) for _ in range(32)]
    require(all(r['outcome'] == 'echo' for r in result['owner_fill']), result)
    result['reserved_sibling_authority'] = lab.expect_sessions(33)
    before = targets[owner].hits
    result['owner_over_share'] = [attempt(lab.ports[owner]) for _ in range(3)]
    require(all(r['outcome'] == 'reset' for r in result['owner_over_share']), result)
    require(targets[owner].hits == before, 'local share denial reached target')
    result['independent_new_clients'] = [attempt(lab.ports['independent']) for _ in range(3)]
    require(all(r['outcome'] == 'echo' for r in result['independent_new_clients']), result)
    result['sibling_fill'] = [attempt(lab.ports[other], conns) for _ in range(32)]
    require(all(r['outcome'] == 'echo' for r in result['sibling_fill']), result)
    result['both_full_authority'] = lab.expect_sessions(65)
    result['both_overflow'] = {name: [attempt(lab.ports[name]) for _ in range(3)]
                               for name in (owner, other)}
    started = time.monotonic()
    pulses = 0
    while time.monotonic() - started < active:
        pulse(lab, conns + control)
        pulses += 1
        lab.wait(min(.25, max(0, active - (time.monotonic() - started))))
    result['active_hold_seconds'] = round(time.monotonic() - started, 3)
    result['active_echo_rounds'] = pulses
    result['both_after_active'] = {name: attempt(lab.ports[name]) for name in (owner, other)}
    require(all(r['outcome'] == 'reset' for rs in result['both_overflow'].values() for r in rs), result)
    require(all(r['outcome'] == 'reset' for r in result['both_after_active'].values()), result)
    pulse(lab, conns + control)
    result['existing_sessions_echo'] = len(conns) + len(control)
    conns.pop(0).close()  # Release one owner's slot, not the sibling's.
    result['one_released_authority'] = lab.expect_sessions(64)
    result['sibling_cannot_borrow'] = attempt(lab.ports[other])
    require(result['sibling_cannot_borrow']['outcome'] == 'reset', result)
    result['owner_refill'] = attempt(lab.ports[owner], conns)
    require(result['owner_refill']['outcome'] == 'echo', result)
    result['refilled_authority'] = lab.expect_sessions(65)
    result['health'] = lab.health()
    require(all(row['last_failure']['class'] == 'dial' for row in result['health']), result)
    for conn in conns:
        conn.close()
    conns.clear()
    result['released_authority'] = lab.expect_sessions(1)
    result['both_after_release'] = {name: attempt(lab.ports[name]) for name in ('left', 'right')}
    require(all(r['outcome'] == 'echo' for r in result['both_after_release'].values()), result)
    lab.expect_sessions(1)
    return result


def scenario(lab, pair, active, expectation):
    targets = {}
    conns, control = [], []
    try:
        targets = {name: Echo() for name in ('left', 'right', 'independent')}
        lab.setup_pair(*pair.split('-'), targets)
        baseline = {name: attempt(lab.ports[name]) for name in ('left', 'right')}
        require(all(row['outcome'] == 'echo' for row in baseline.values()), baseline)
        independent = attempt(lab.ports['independent'], control)
        require(independent['outcome'] == 'echo', independent)
        lab.expect_sessions(1)
        result = {'pair': pair, 'baseline': baseline, 'independent_peer': independent}
        run_phase = phase_shares if expectation == 'shares' else phase
        result['phases'] = [run_phase(lab, owner, other, targets, active, conns, control)
                            for owner, other in (('left', 'right'), ('right', 'left'))]
        result['daemons_alive'] = all(p.poll() is None for p in lab.wolves)
        require(result['daemons_alive'], 'daemon exited')
        result['target_payload_streams'] = {name: target.hits for name, target in targets.items()}
        result['daemon_sha256'] = lab.binary_hash
        result['receiver_pid'] = lab.wolves[0].pid
        result['initiator_pid'] = lab.wolves[1].pid
        result['log_bytes'] = {p.name: p.stat().st_size for p in lab.base.glob('*.log')}
        result['status'] = 'PASS' if expectation == 'shares' else 'CONFIRMED'
        return result
    finally:
        for conn in conns + control:
            conn.close()
        for target in targets.values():
            target.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('pairs', nargs='*', help='default: all four QUIC/TCP pairs')
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--active', type=float, default=5)
    parser.add_argument('--expect', choices=('baseline', 'shares'), default='baseline')
    parser.add_argument('--out', type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.pairs = args.pairs or list(PAIRS)
    require(all(pair in PAIRS for pair in args.pairs), f'unknown pair; choose from {PAIRS}')
    require(args.rounds > 0 and args.active >= 1, 'rounds must be positive; active >= 1 s')
    args.out.parent.mkdir(parents=True, exist_ok=True)
    # Never truncate an older run, including a failed development smoke.
    with args.out.open('x') as output:
        output.write('[]\n')
    os.umask(0o077)
    inputs = [pathlib.Path(__file__).resolve(), ROOT / 'tools/stage42/fang_flood.py',
              ROOT / 'tests/integration/run.py', ROOT / 'Cargo.lock']
    metadata = {'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT,
                                                 text=True).strip(),
                'input_sha256': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                                 for p in inputs}}
    rows = []
    for repetition in range(1, args.rounds + 1):
        for pair in args.pairs:
            lab = QuotaLab()
            row = {'round': repetition, 'pair': pair, 'expectation': args.expect, **metadata}
            try:
                row.update(scenario(lab, pair, args.active, args.expect))
            except Exception as exc:
                row.update(status='FAIL', error=f'{type(exc).__name__}: {exc}')
                row['failure_logs'] = {p.name: p.read_text(errors='replace')[-2000:]
                                       for p in lab.base.glob('*.log')}
            finally:
                row['daemon_sha256'] = getattr(lab, 'binary_hash', None)
                try:
                    lab.cleanup()
                    require(not lab.base.exists(), 'private lab directory remains')
                    require(all(p.poll() is not None for p in lab.processes),
                            'owned child process remains')
                    row['cleanup'] = 'PASS'
                    row['private_fixture_removed'] = True
                except Exception as exc:
                    row.update(status='FAIL', cleanup_error=f'{type(exc).__name__}: {exc}')
            rows.append(row)
            args.out.write_text(json.dumps(rows, indent=2) + '\n')
            print(json.dumps(row), flush=True)
    expected = 'PASS' if args.expect == 'shares' else 'CONFIRMED'
    return 0 if all(row['status'] == expected for row in rows) else 1


if __name__ == '__main__':
    sys.exit(main())
