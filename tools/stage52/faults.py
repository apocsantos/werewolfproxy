#!/usr/bin/env python3
"""Stage52 M3: disposable resolver failure, job-lifetime and replay controls.

Only the lab initiator loads resolver_shim.c. Missing/blocked answers are
fault injection, not a real DNS/TTL experiment. Wrong-key cases use real
hosts-file answers and a different authenticated receiver at the SAME port.
All hosts, identities, routes and processes belong to Stage46's namespace.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
import pathlib
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time

sys.dont_write_bytecode = True
import refresh
from refresh import ROOT, OLD, NAME, dial, require

WRONG = '127.0.0.54'
PATHS = ('quic', 'routed-quic', 'failover-quic', 'failover-routed-quic')
SCENARIOS = tuple(f'{fault}:{path}' for fault in ('missing', 'wrong-key') for path in PATHS) + (
    'coalesced', 'capacity', 'post-ack:failover-quic', 'post-ack:failover-routed-quic')


def client(port):
    started = time.monotonic()
    outcome = dial.attempt(port, timeout=12)
    return [outcome, round(time.monotonic() - started, 3)]


class ControlledHandler(refresh.EchoHandler):
    def handle(self):
        self.request.settimeout(.05)
        with self.server.lock:
            self.server.connections.add(self.request)
        counted = False
        try:
            while not self.server.abort.is_set():
                try:
                    data = self.request.recv(65536)
                except socket.timeout:
                    continue
                if not data:
                    break
                if not counted:
                    with self.server.lock:
                        self.server.hits += 1
                    counted = True
                self.request.sendall(data)
        except OSError:
            pass
        finally:
            if self.server.abort.is_set():
                self.request.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack('ii', 1, 0))
            self.request.close()
            with self.server.lock:
                self.server.connections.discard(self.request)


class ControlledEcho(refresh.CountedEcho):
    def __init__(self):
        self.abort = threading.Event()
        self.connections = set()
        super().__init__()
        self.RequestHandlerClass = ControlledHandler


class FaultLab(dial.DialLab):
    def __init__(self, library):
        super().__init__()
        self.mode_path = self.base / 'resolver.mode'
        self.mode_path.write_bytes(b'o' + bytes(4095))
        self.resolver_log = self.base / 'resolver.log'
        self.library = self.base / 'resolver.so'
        shutil.copy2(library, self.library)

    def reserve_wolf_ports(self):
        super().reserve_wolf_ports()
        # Different identities B and C share a numeric service port on
        # different loopback addresses: a genuine wrong-IP answer suffices.
        for key in ('c_tcp', 'c_quic'):
            self.release(key)
            self.ports[key] = self.ports['b_quic']

    def address(self, key):
        host = OLD if key.startswith('b_') else WRONG if key.startswith('c_') else '127.0.0.1'
        return f'{host}:{self.ports[key]}'

    def listener(self, key):
        host, port = self.address(key).rsplit(':', 1)
        with socket.create_connection((host, int(port)), timeout=1):
            pass

    def spawn(self, argv, label, env=None):
        if label.startswith('a-') and str(argv[0]) == str(self.binary):
            env = dict(env or self.env, LD_PRELOAD=str(self.library),
                       STAGE52_RESOLVER_MODE=str(self.mode_path),
                       STAGE52_RESOLVER_LOG=str(self.resolver_log))
        return super().spawn(argv, label, env)

    @property
    def a_pid(self):
        return self.wolves[0].pid

    def mode(self, value):
        with self.mode_path.open('r+b') as handle:
            handle.write(value.encode('ascii'))

    def events(self):
        if not self.resolver_log.exists():
            return []
        text = self.resolver_log.read_text()
        lines = text.splitlines()
        if text and not text.endswith('\n'):
            lines = lines[:-1]
        return [json.loads(line) for line in lines]

    def live_jobs(self):
        events = self.events()
        return len({e['id'] for e in events if e['event'] == 'start'} -
                   {e['id'] for e in events if e['event'] == 'end'})

    def until(self, predicate, description, seconds=3):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.guard()
            require(all(p.poll() is None for p in self.wolves), 'fixture daemon exited')
            if predicate():
                return
            time.sleep(.01)
        raise RuntimeError(f'timed out waiting for {description}')

    def reset_events(self):
        self.until(lambda: self.live_jobs() == 0, 'preflight resolver drain')
        time.sleep(.25)
        require(self.live_jobs() == 0, 'preflight lookup still running')
        self.resolver_log.write_text('')

    def cpu_ticks(self):
        fields = pathlib.Path(f'/proc/{self.a_pid}/stat').read_text().rsplit(')', 1)[1].split()
        return int(fields[11]) + int(fields[12])

    def fds(self):
        return len(list(pathlib.Path(f'/proc/{self.a_pid}/fd').iterdir()))

    def open_extra(self, service):
        key = f'extra-{len(self.ports)}'
        self.reserve(key)
        self.release(key)
        opened = self.ctl_any('a', 'service', 'connect', service, '--local-port',
                              self.ports[key], '--transport', 'quic')
        require(opened['exit'] == 0, f'extra Fang failed: {opened}')
        self.wait_for(lambda: self.listener(key), key)
        return {'id': opened['body']['result']['fang_id'], 'port': self.ports[key]}

    def close_fang(self, fang):
        self.control('a', 'fang.close', {'fang_id': fang['id']})


def prepare(lab, path):
    refresh.map_name(OLD)
    lab.boot()
    lab.targets = {}
    for wolf in lab.WOLVES:
        if wolf != 'a':
            lab.targets[wolf] = ControlledEcho()
    targets = lab.targets
    transport = refresh.configure(lab, path, targets)
    opened = lab.connect(transport)
    require(opened['exit'] == 0, f'Fang open failed: {opened}')
    fang = {'id': opened['body']['result']['fang_id'], 'port': lab.ports['fang']}
    provider = 'c' if path in ('routed-quic', 'failover-routed-quic') else 'b'
    return targets, fang, provider


def checked_events(lab):
    events = lab.events()
    require(events and all(e['pid'] == lab.a_pid for e in events), 'shim did not run only in A')
    require(not any(e['event'] == 'guard' for e in events), 'fixture 30 s hold guard triggered')
    return events


def phase(lab, path, fault, count, result, targets, provider):
    result['before'] = refresh.measure(lab, count, targets)
    refresh.require_echo(result['before'], count, provider)
    with socket.create_connection(('127.0.0.1', lab.ports['fang']), timeout=12) as held:
        held.settimeout(12)
        refresh.echo_held(held)
        lab.reset_events()
        if fault == 'missing':
            lab.mode('n')
        else:
            # C is reachable at the SAME port with a different Pack key.
            # Grant possible targets too, so target denial cannot mask an
            # identity/authorization bypass in this adversarial fixture.
            lab.ctl('c', 'target', 'allow', 'wolf-a', targets['b'].address)
            lab.ctl('c', 'target', 'allow', 'wolf-a', lab.address('c_tcp'))
            refresh.map_name(WRONG)
        result['fault'] = refresh.measure(lab, count, targets)
        changed = result['fault']
        if path.startswith('failover'):
            refresh.require_echo(changed, count, 'd' if path == 'failover-routed-quic' else 'c')
        else:
            require(all(o == 'reset' for o, _ in changed['clients']), f'fault did not reset: {changed}')
            require(not any(changed['target_connections'].values()), 'fault reached an application target')
            failure = next(f for f in changed['fang_list'] if f['local'] == lab.address('fang'))
            require(failure['last_failure']['class'] == ('dial' if fault == 'missing' else 'handshake'),
                    f'wrong failure class: {failure}')
        refresh.echo_held(held)
        result['held_during_fault'] = 'echo'
        lab.mode('o')
        refresh.map_name(OLD)
        result['recovered'] = refresh.measure(lab, count, targets)
        refresh.require_echo(result['recovered'], count, provider)
        refresh.echo_held(held)
        result['held_after_recovery'] = 'echo'
    result['resolver_events'] = checked_events(lab)


def flood(lab, global_capacity, result, targets, fang):
    lab.ctl('a', 'service', 'add', 'control', 'wolf-c', targets['c'].address, 'tcp')
    literal = lab.open_extra('control')
    fangs = [fang]
    if global_capacity:
        fangs += [lab.open_extra('echo') for _ in range(16)]
    result['fds_before'] = lab.fds()
    with socket.create_connection(('127.0.0.1', fang['port']), timeout=12) as held:
        held.settimeout(12)
        refresh.echo_held(held)
        lab.reset_events()
        lab.mode('b')
        wanted = 16 if global_capacity else 1
        # Keep room for the held-session control under the existing
        # 64-connection Fang ceiling; this case tests resolver coalescing.
        ports = [f['port'] for f in fangs] if global_capacity else [fang['port']] * 48
        before = {key: target.hits for key, target in targets.items()}
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(ports)) as pool:
            pending = [pool.submit(client, port) for port in ports]
            lab.until(lambda: lab.live_jobs() == wanted, f'{wanted} blocked resolver jobs')
            result['literal_during_saturation'] = [client(literal['port']) for _ in range(3)]
            require(all(o == 'echo' for o, _ in result['literal_during_saturation']),
                    'literal Fang was blocked by resolver capacity')
            result['clients'] = [f.result(timeout=12) for f in pending]
        require(all(o == 'reset' and 4.5 <= elapsed < 5.5 for o, elapsed in result['clients']),
                f'blocked clients did not meet their dial cutoff: {result["clients"]}')
        require(targets['b'].hits == before['b'], 'blocked new clients reached primary target')
        require(lab.live_jobs() == wanted, 'caller timeout released a still-blocked resolver job')
        refresh.echo_held(held)
        result['held_during_saturation'] = 'echo'
    for entry in fangs:
        lab.close_fang(entry)
    require(lab.live_jobs() == wanted, 'Fang closure ended non-cancellable blocking work')
    result['jobs_after_timeouts_and_fang_close'] = lab.live_jobs()
    result['fds_while_jobs_outlive_fangs'] = lab.fds()
    start = lab.cpu_ticks()
    began = time.monotonic()
    time.sleep(2)
    result['blocked_cpu_pct'] = round(100 * (lab.cpu_ticks() - start) /
                                     os.sysconf('SC_CLK_TCK') / (time.monotonic() - began), 2)
    require(result['blocked_cpu_pct'] < 10, 'sustained CPU spin while resolver work blocked')
    result['literal_after_fang_close'] = [client(literal['port']) for _ in range(3)]
    require(all(o == 'echo' for o, _ in result['literal_after_fang_close']), 'literal recovery failed')
    events = checked_events(lab)
    starts = [e for e in events if e['event'] == 'start']
    require(len(starts) == wanted and max(e['active'] for e in starts) == wanted,
            f'resolver cap/coalescing failed: {events}')
    lab.mode('o')
    lab.until(lambda: lab.live_jobs() == 0, 'actual blocking completion')
    time.sleep(.1)
    result['resolver_events'] = checked_events(lab)
    require(sum(e['event'] == 'start' for e in result['resolver_events']) == wanted,
            'a cancelled capacity waiter submitted a late job')
    require(sum(e['event'] == 'end' for e in result['resolver_events']) == wanted,
            'blocking resolver jobs did not all finish')
    result['fds_after_close'] = lab.fds()
    if global_capacity:
        # A single healthy connection would only prove ONE slot recovered.
        # Refill all 16, then close their Fangs before releasing the jobs.
        refilled = [lab.open_extra('echo') for _ in range(16)]
        lab.reset_events()
        lab.mode('b')
        with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
            pending = [pool.submit(client, f['port']) for f in refilled]
            lab.until(lambda: lab.live_jobs() == 16, 'all 16 recovered capacity slots')
            for entry in refilled:
                lab.close_fang(entry)
            require(all(f.result(timeout=12)[0] == 'reset' for f in pending),
                    'closing refilled Fangs did not reset their clients')
        require(lab.live_jobs() == 16, 'refilled jobs ended on Fang closure')
        lab.mode('o')
        lab.until(lambda: lab.live_jobs() == 0, 'refilled blocking completion')
        events = checked_events(lab)
        require(sum(e['event'] == 'start' for e in events) == 16 and
                sum(e['event'] == 'end' for e in events) == 16,
                'refilled capacity did not complete exactly 16 jobs')
        result['refilled_capacity_events'] = events
    recovered = lab.open_extra('echo')
    result['recovered'] = [client(recovered['port']) for _ in range(3)]
    require(all(o == 'echo' for o, _ in result['recovered']), 'resolver capacity did not recover')


def post_ack(lab, path, result, targets, fang, provider):
    fallback = 'd' if path == 'failover-routed-quic' else 'c'
    with socket.create_connection(('127.0.0.1', fang['port']), timeout=12) as conn:
        conn.settimeout(12)
        refresh.echo_held(conn)
        fallback_before = targets[fallback].hits
        lab.mode('n')
        refresh.echo_held(conn)
        targets[provider].abort.set()
        try:
            data = conn.recv(1)
            result['committed_end'] = 'eof' if not data else 'unexpected-data'
        except ConnectionResetError:
            result['committed_end'] = 'reset'
        require(result['committed_end'] == 'reset', 'committed failure was not visible as reset')
        time.sleep(.1)
        require(targets[fallback].hits == fallback_before, 'data replayed through fallback after ACK')
        result['fallback_connections_after_committed_failure'] = targets[fallback].hits - fallback_before
    # Prove the fallback is available to a NEW connection before commitment.
    result['new_connection_during_fault'] = refresh.measure(lab, 1, targets)
    refresh.require_echo(result['new_connection_during_fault'], 1, fallback)
    targets[provider].abort.clear()
    lab.mode('o')
    result['recovered'] = refresh.measure(lab, 1, targets)
    refresh.require_echo(result['recovered'], 1, provider)
    result['resolver_events'] = checked_events(lab)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('scenarios', nargs='*', help=', '.join(SCENARIOS))
    parser.add_argument('--rounds', type=int, default=1)
    parser.add_argument('--attempts', type=int, default=3)
    parser.add_argument('--out', required=True)
    args = parser.parse_args()
    require(args.rounds > 0 and args.attempts > 0, 'positive rounds/attempts required')
    require(all(s in SCENARIOS for s in args.scenarios), 'unknown scenario')
    out = pathlib.Path(args.out).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    require(not out.exists(), f'refusing to overwrite {out}')
    library = out.parent / 'resolver_shim.so'
    if os.environ.get('WWP_STAGE46_SANDBOX') != '1':
        subprocess.run(['cc', '-shared', '-fPIC', '-O2', '-std=c11', '-Wall', '-Wextra', '-Werror',
                        str(ROOT / 'tools/stage52/resolver_shim.c'), '-o', str(library),
                        '-ldl', '-pthread'], check=True)
        env = dict(os.environ, WWP_STAGE46_SANDBOX='1')
        os.execvpe('unshare', ['unshare', '-rmn', '--fork', '--kill-child', '--', sys.executable,
                              '-B', str(pathlib.Path(__file__).resolve()), *sys.argv[1:]], env)
    dial.enter_sandbox()
    os.umask(0o077)
    report = {'status': 'RUNNING', 'results': [], 'rounds': args.rounds,
              'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
              'shim_sha256': hashlib.sha256(library.read_bytes()).hexdigest(),
              'source_hashes': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in
                                (pathlib.Path(__file__), ROOT / 'tools/stage52/resolver_shim.c')}}
    refresh.save(out, report)
    for round_number in range(1, args.rounds + 1):
        for scenario in args.scenarios or SCENARIOS:
            print(f'round {round_number}: {scenario} starting', flush=True)
            lab = FaultLab(library)
            path = scenario.split(':', 1)[1] if ':' in scenario else 'quic'
            if path == 'failover-routed-quic':
                lab.WOLVES = ('a', 'b', 'c', 'd')
            targets = {}
            result = {'round': round_number, 'scenario': scenario, 'status': 'FAIL'}
            started = time.monotonic()
            try:
                targets, fang, provider = prepare(lab, path)
                if scenario in ('coalesced', 'capacity'):
                    flood(lab, scenario == 'capacity', result, targets, fang)
                elif scenario.startswith('post-ack:'):
                    post_ack(lab, path, result, targets, fang, provider)
                else:
                    phase(lab, path, scenario.split(':')[0], args.attempts, result, targets, provider)
                result['status'] = 'PASS'
                result['daemons_alive'] = all(p.poll() is None for p in lab.wolves)
            except Exception as exc:
                result['error'] = f'{type(exc).__name__}: {exc}'
            finally:
                lab.mode('o')
                result['daemon_sha256'] = getattr(lab, 'binary_hash', None)
                errors = []
                for target in (targets or getattr(lab, 'targets', {})).values():
                    try:
                        target.abort.set()
                        target.close()
                    except Exception as exc:
                        errors.append(f'{type(exc).__name__}: {exc}')
                try:
                    lab.cleanup()
                except Exception as exc:
                    errors.append(f'{type(exc).__name__}: {exc}')
                result['cleanup'] = errors or 'PASS'
                if errors:
                    result['status'] = 'FAIL'
                result['seconds'] = round(time.monotonic() - started, 2)
                report['results'].append(result)
                refresh.save(out, report)
            print(f'round {round_number}: {scenario} {result["status"]} {result.get("error", "")}',
                  flush=True)
            if result['status'] != 'PASS':
                report['status'] = 'FAIL'
                refresh.save(out, report)
                return 1
    report['status'] = 'PASS'
    refresh.save(out, report)
    return 0


if __name__ == '__main__':
    sys.exit(main())
