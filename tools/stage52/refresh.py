#!/usr/bin/env python3
"""Stage52: reproduce QUIC's open-time hostname snapshot, without a fix.

Uses Stage46's private user/mount/network namespace and disposable lab.
Changes only the private /etc/hosts mapping, then makes the old loopback
address unreachable in that namespace. This tests resolver freshness,
not authoritative DNS, TTLs or a real WAN address migration.
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
sys.path.insert(0, str(ROOT / 'tools/stage46'))
import dial  # noqa: E402
from run import require  # noqa: E402

NAME = 'moving.stage52'
OLD = '127.0.0.52'
NEW = '127.0.0.53'
PAYLOAD = b'stage52-held-echo'
PATHS = ('quic', 'tcp', 'routed-quic', 'routed-tcp', 'failover-quic', 'failover-tcp')


def resolve():
    return list(dict.fromkeys(info[4][0] for info in socket.getaddrinfo(
        NAME, 1, type=socket.SOCK_STREAM)))


def map_name(address):
    # Stage46's sandbox bind-mounts this file over /etc/hosts. Write the
    # same inode; replacing the pathname would not change the bind mount.
    path = pathlib.Path('/hosts')
    text = '\n'.join(line for line in path.read_text().splitlines() if NAME not in line)
    path.write_text(text + f'\n{address}\t{NAME}\n')
    require(resolve() == [address], f'private resolver did not switch to {address}: {resolve()}')


class EchoHandler(socketserver.BaseRequestHandler):
    def handle(self):
        counted = False
        self.request.settimeout(20)
        try:
            while data := self.request.recv(65536):
                if not counted:
                    with self.server.lock:
                        self.server.hits += 1
                    counted = True
                self.request.sendall(data)
        except OSError:
            pass


class CountedEcho(socketserver.ThreadingTCPServer):
    daemon_threads = True
    block_on_close = False

    def __init__(self):
        self.hits = 0
        self.lock = threading.Lock()
        super().__init__(('127.0.0.1', 0), EchoHandler)
        self.thread = threading.Thread(target=self.serve_forever,
                                       kwargs={'poll_interval': .05}, daemon=True)
        self.thread.start()

    @property
    def address(self):
        return f'127.0.0.1:{self.server_address[1]}'

    def close(self):
        self.shutdown()
        self.server_close()
        self.thread.join(timeout=2)
        require(not self.thread.is_alive(), 'echo server did not stop')


class RefreshLab(dial.DialLab):
    def start_wolves(self):
        # B serves both loopback aliases with ONE process/identity. No
        # restart, duplicate Den or packet-forwarding proxy is involved.
        self.binding_receivers = True
        try:
            super().start_wolves()
        finally:
            self.binding_receivers = False

    def address(self, key):
        if getattr(self, 'binding_receivers', False) and key in ('b_tcp', 'b_quic'):
            return f'0.0.0.0:{self.ports[key]}'
        return super().address(key)


def echo_held(conn):
    conn.sendall(PAYLOAD)
    got = b''
    while len(got) < len(PAYLOAD):
        data = conn.recv(len(PAYLOAD) - len(got))
        require(data, 'held connection ended')
        got += data
    require(got == PAYLOAD, 'held connection payload mismatch')


def configure(lab, path, targets):
    scheme = 'tcp' if path.endswith('tcp') else 'quic'
    for wolf in ('b', 'c'):
        lab.pack_literal(wolf, 'a')
    added = lab.pack('a', 'b', f'{scheme}://{NAME}:{lab.ports["b_quic"]}')
    require(added['exit'] == 0, f'hostname Pack add failed: {added}')
    lab.pack_literal('a', 'c', 'tcp' if path.startswith('routed') else scheme)
    for wolf in ('b', 'c'):
        lab.ctl(wolf, 'target', 'allow', 'wolf-a', targets[wolf].address)
    if path.startswith('routed'):
        lab.ctl('a', 'service', 'add', 'echo', 'wolf-c', targets['c'].address, 'tcp')
        lab.ctl('a', 'route', 'set', 'wolf-c', '--via', 'wolf-b', '--hop', scheme)
        lab.ctl('b', 'target', 'allow', 'wolf-a', lab.address('c_tcp'))
        return 'tcp'
    lab.ctl('a', 'service', 'add', 'echo', 'wolf-b', targets['b'].address, 'tcp')
    if path.startswith('failover'):
        lab.ctl('a', 'service', 'provider', 'add', 'echo', 'wolf-c', targets['c'].address)
    return scheme


def measure(lab, count, targets):
    before = {wolf: target.hits for wolf, target in targets.items()}
    result = lab.measure_timed(count)
    result['target_connections'] = {wolf: target.hits - before[wolf]
                                    for wolf, target in targets.items()}
    require(result['daemons_alive'], 'fixture daemon exited')
    return result


def require_echo(result, count, provider):
    require(result['echo'] == count, f'expected {count} echoes: {result["clients"]}')
    expected = {wolf: count if wolf == provider else 0 for wolf in ('b', 'c')}
    require(result['target_connections'] == expected,
            f'wrong provider target reached: {result["target_connections"]}, expected {expected}')


def scenario(lab, path, count):
    result = {'path': path}
    targets = {}
    route_added = False
    held = None
    try:
        map_name(OLD)
        lab.boot()
        targets = {wolf: CountedEcho() for wolf in ('b', 'c')}
        transport = configure(lab, path, targets)
        result['pack_hostname'] = f'{NAME}:{lab.ports["b_quic"]}'
        result['receiver_pid'] = lab.wolves[lab.WOLVES.index('b')].pid
        result['resolved_before'] = resolve()
        opened = lab.connect(transport)
        require(opened['exit'] == 0, f'initial Fang open failed: {opened}')
        result['initial_open'] = opened['body']['result']
        provider = 'c' if path.startswith('routed') else 'b'
        result['before_change'] = measure(lab, count, targets)
        require_echo(result['before_change'], count, provider)

        held = socket.create_connection(('127.0.0.1', lab.ports['fang']), timeout=15)
        held.settimeout(15)
        echo_held(held)
        map_name(NEW)
        result['resolved_after'] = resolve()
        echo_held(held)
        result['established_session_after_mapping_change'] = 'echo'
        held.close()
        held = None
        result['after_mapping_change'] = measure(lab, count, targets)
        require_echo(result['after_mapping_change'], count, provider)

        # Old route is removed only AFTER the established-session control.
        # A private more-specific route overrides loopback's local /8.
        dial.sh('ip', 'route', 'add', 'unreachable', OLD + '/32', 'table', 'local')
        route_added = True
        try:
            with socket.create_connection((OLD, lab.ports['b_tcp']), timeout=1):
                raise RuntimeError('old address is still reachable')
        except OSError as exc:
            result['old_address_error'] = {'type': type(exc).__name__, 'errno': exc.errno}
        with socket.create_connection((NEW, lab.ports['b_tcp']), timeout=1):
            result['new_address_listener'] = 'reachable'
        result['after_old_address_withdrawal'] = measure(lab, count, targets)
        changed = result['after_old_address_withdrawal']
        if path.endswith('tcp'):
            require_echo(changed, count, provider)
            result['finding'] = 'FRESH_TCP_CONTROL'
        elif path == 'failover-quic':
            require_echo(changed, count, 'c')
            result['finding'] = 'STALE_QUIC_PRIMARY_SKIPPED'
        else:
            require(all(outcome == 'reset' for outcome, _ in changed['clients']),
                    f'expected stale-QUIC resets: {changed["clients"]}')
            require(changed['target_connections'] == {'b': 0, 'c': 0}, 'stale dial reached target')
            result['finding'] = 'STALE_QUIC_RESET'

        entries = [f for f in lab.fang_list() if f['local'] == lab.address('fang')]
        require(len(entries) == 1, f'expected exactly one Fang: {entries}')
        lab.control('a', 'fang.close', {'fang_id': entries[0]['id']})
        reopened = lab.connect(transport)
        require(reopened['exit'] == 0, f'reopen failed: {reopened}')
        result['reopened'] = measure(lab, count, targets)
        require_echo(result['reopened'], count, provider)
        require(lab.wolves[lab.WOLVES.index('b')].pid == result['receiver_pid'],
                'receiver changed during scenario')
        result['status'] = 'PASS'
    except Exception as exc:
        result['status'] = 'FAIL'
        result['error'] = f'{type(exc).__name__}: {exc}'
    finally:
        if held is not None:
            held.close()
        for target in targets.values():
            target.close()
        if route_added:
            dial.sh('ip', 'route', 'del', 'unreachable', OLD + '/32', 'table', 'local')
    return result


def save(out, report):
    pending = out.with_suffix('.tmp')
    pending.write_text(json.dumps(report, indent=2) + '\n')
    pending.replace(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('paths', nargs='*', help=', '.join(PATHS) + ' (default: all)')
    parser.add_argument('--attempts', type=int, default=3)
    parser.add_argument('--rounds', type=int, default=1)
    parser.add_argument('--out', required=True)
    args = parser.parse_args()
    require(args.attempts > 0 and args.rounds > 0, 'attempts and rounds must be positive')
    require(all(path in PATHS for path in args.paths), 'unknown scenario path')
    out = pathlib.Path(args.out).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    require(not out.exists(), f'refusing to overwrite evidence: {out}')
    # Re-exec THIS tool; Stage46's own entry point has different arguments.
    if os.environ.get('WWP_STAGE46_SANDBOX') != '1':
        env = dict(os.environ, WWP_STAGE46_SANDBOX='1')
        os.execvpe('unshare', ['unshare', '-rmn', '--fork', '--kill-child', '--',
                              sys.executable, '-B', str(pathlib.Path(__file__).resolve()),
                              *sys.argv[1:]], env)
    dial.enter_sandbox()
    os.umask(0o077)
    report = {
        'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT,
                                                 text=True).strip(),
        'tool_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
        'lock_sha256': hashlib.sha256((ROOT / 'Cargo.lock').read_bytes()).hexdigest(),
        'fixture': 'private /etc/hosts, same receiver on both aliases; old route withdrawn',
        'attempts': args.attempts, 'rounds': args.rounds, 'status': 'RUNNING', 'results': [],
    }
    save(out, report)
    for round_number in range(1, args.rounds + 1):
        for path in args.paths or PATHS:
            print(f'round {round_number}: {path} starting', flush=True)
            lab = RefreshLab()
            started = time.monotonic()
            result = {'path': path, 'status': 'FAIL', 'error': 'scenario did not finish'}
            try:
                result = scenario(lab, path, args.attempts)
            finally:
                result['daemon_sha256'] = getattr(lab, 'binary_hash', None)
                try:
                    lab.cleanup()
                    result['cleanup'] = 'PASS'
                except Exception as exc:
                    result.update(status='FAIL', cleanup=f'{type(exc).__name__}: {exc}')
                result['round'] = round_number
                result['seconds'] = round(time.monotonic() - started, 2)
                report['results'].append(result)
                save(out, report)
            print(f'round {round_number}: {path} {result["status"]}: '
                  f'{result.get("finding", result.get("error"))}', flush=True)
            if result['status'] != 'PASS':
                report['status'] = 'FAIL'
                save(out, report)
                return 1
    report['status'] = 'PASS'
    save(out, report)
    return 0


if __name__ == '__main__':
    sys.exit(main())
