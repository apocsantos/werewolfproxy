#!/usr/bin/env python3
"""Stage54 M3 idle-boundary, routed-pressure and source-budget regressions.

Fresh checkout-only labs, real 65-second idle intervals, generated identities,
dynamic ports, 50 MiB log guards and owned-process cleanup. Source cases use
only a private user/mount/network namespace. No installed nodes or keep-temp.
"""
import argparse
import collections
import concurrent.futures
import json
import os
import pathlib
import socket
import socketserver
import subprocess
import sys
import time

sys.dont_write_bytecode = True
import idle_share as idle
from idle_share import support, require, open_fang, until
from peer_quota import exchange, PAYLOAD

CASES = tuple(f'{case}-{transport}' for case in
              ('boundary', 'relay', 'destination', 'source') for transport in ('quic', 'tcp'))
HOST = '10.54.0.1'
SOURCES = ('10.54.0.2', '10.54.0.3', '10.54.0.4')


class HalfHandler(socketserver.BaseRequestHandler):
    """Echo once, finish downstream, retain the upload half until cleanup."""
    def handle(self):
        self.request.settimeout(.05)
        with self.server.lock:
            self.server.connections.add(self.request)
        first, pending = True, b''
        try:
            while not self.server.abort.is_set():
                try:
                    data = self.request.recv(65536)
                except socket.timeout:
                    continue
                if not data:
                    break
                if first:
                    pending += data
                    if len(pending) < len(PAYLOAD):
                        continue
                    with self.server.lock:
                        self.server.hits += 1
                    self.request.sendall(pending)
                    self.request.shutdown(socket.SHUT_WR)
                    first = False
        except OSError:
            pass
        finally:
            self.request.close()
            with self.server.lock:
                self.server.connections.discard(self.request)


class HalfEcho(support.faults.ControlledEcho):
    def __init__(self):
        super().__init__()
        self.RequestHandlerClass = HalfHandler


class Scenario:
    def __init__(self, lab, wolves, half=False):
        self.lab = lab
        self.connections = []
        self.targets = {w: (HalfEcho() if half and w == 'g' else
                            support.faults.ControlledEcho()) for w in wolves}
        lab.boot()
        self.baseline = idle.fd_count(lab)
        for wolf, target in self.targets.items():
            require(lab.pack_literal('a', wolf)['exit'] == 0, 'initiator Pack setup')
            require(lab.pack_literal(wolf, 'a')['exit'] == 0, 'receiver Pack setup')
            lab.ctl(wolf, 'target', 'allow', 'wolf-a', target.address)
            lab.ctl('a', 'service', 'add', 'echo-' + wolf, 'wolf-' + wolf,
                    target.address, 'tcp')

    def open(self, wolf, transport, source=False, service=None):
        service = service or 'echo-' + wolf
        if not source:
            return open_fang(self.lab, service, transport)
        self.lab.reserve('extra')
        port = self.lab.ports['extra']
        self.lab.release('extra')
        opened = self.lab.ctl_any('a', 'service', 'connect', service,
                                  '--local-port', port, '--transport', transport,
                                  '--bind', HOST)
        require(opened['exit'] == 0, opened)
        return {'port': port, 'id': opened['body']['result']['fang_id'], 'host': HOST}

    def probe(self, fang, retained=None, source=None):
        started, conn = time.monotonic(), None
        try:
            conn = socket.create_connection((fang.get('host', '127.0.0.1'), fang['port']),
                                            timeout=5,
                                            source_address=(source, 0) if source else None)
            conn.settimeout(5)
            exchange(conn)
            outcome = 'echo'
            if retained is not None:
                retained.append(conn)
                self.connections.append(conn)
                conn = None
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

    def fill(self, fang, count, source=None):
        result = []
        for _ in range(count):
            self.lab.guard()
            probe = self.probe(fang, result, source)
            require(probe['outcome'] == 'echo', f'fill failed at {len(result)} / {count}: {probe}')
        return result

    def deny(self, fang, source=None, count=3):
        rows = [self.probe(fang, source=source) for _ in range(count)]
        require(all(r['outcome'] == 'reset' for r in rows), rows)
        return rows

    def wait(self, controls, phase):
        phase('idle-wait', {'seconds': idle.IDLE})
        started, pulses = time.monotonic(), 0
        while time.monotonic() - started < idle.IDLE:
            self.lab.guard()
            require(all(p.poll() is None for p in self.lab.wolves), 'fixture daemon exited')
            support.pulse(self.lab, controls)
            pulses += 1
            time.sleep(min(1, max(0, idle.IDLE - (time.monotonic() - started))))
        elapsed = time.monotonic() - started
        require(elapsed >= idle.IDLE, 'shortened idle observation')
        return {'idle_seconds': round(elapsed, 3), 'control_echo_rounds': pulses}

    def finish(self, independent, controls):
        support.pulse(self.lab, controls)
        independent_new = [self.probe(independent) for _ in range(3)]
        require(all(r['outcome'] == 'echo' for r in independent_new), 'independent admission')
        cpu = support.idle_cpu(self.lab)
        for fang in self.lab.fang_list():
            self.lab.control('a', 'fang.close', {'fang_id': fang['id']})
        for conn in self.connections:
            conn.close()
        final = {w: support.expect(self.lab, w, 0) for w in self.targets}
        until(self.lab, lambda: idle.fd_count(self.lab) <= self.baseline, 'descriptors drained')
        for target in self.targets.values():
            until(self.lab, lambda target=target: not target.connections, 'target handlers drained')
        require(all(p.poll() is None for p in self.lab.wolves), 'fixture daemon exited')
        return {'independent_new_clients': independent_new, 'independent_held_echo': True,
                'idle_cpu_pct': cpu, 'final_authority': final, 'target_handlers': 0,
                'fds_before': self.baseline, 'fds_after': idle.fd_count(self.lab),
                'daemons_alive': True}

    def close(self):
        for conn in self.connections:
            conn.close()
        for target in self.targets.values():
            target.close()


def replacement(lab, old, count):
    # Cancellation and its client RST are asynchronous. Retain terminal
    # observations: reading SO_ERROR via recv would consume a pending reset.
    states = ['open'] * len(old)
    def observed():
        for index, conn in enumerate(old):
            if states[index] == 'open':
                states[index] = idle.states([conn])[0]
        require(all(state in ('open', 'reset') for state in states) and
                states.count('reset') <= count, f'unexpected idle outcomes: {states}')
        return states.count('reset') == count
    until(lab, observed, f'exactly {count} old idle client resets')
    return [conn for conn, state in zip(old, states) if state == 'open']


def provider_pair(s, primary, fallback, transport, source=False):
    service = 'pair-' + primary
    s.lab.ctl('a', 'service', 'add', service, 'wolf-' + primary,
              s.targets[primary].address, 'tcp')
    s.lab.ctl('a', 'service', 'provider', 'add', service,
              'wolf-' + fallback, s.targets[fallback].address)
    main = s.open(primary, transport, source, service)
    # First member receives the 22-slot remainder at both receiver keys.
    for wolf in (primary, fallback):
        for _ in range(2):
            s.open(wolf, transport, source)
    return main


def stop_listener(target):
    # Keep existing authenticated streams alive: only future target connects fail.
    target.shutdown()
    target.server_close()
    target.thread.join(timeout=2)
    require(not target.thread.is_alive(), 'target listener failed to stop')


def boundary(s, transport, repetition, phase):
    lab = s.lab
    other = 'tcp' if transport == 'quic' else 'quic'
    fangs = [s.open('b', t) for t in (transport, other, transport)]
    owner = s.fill(fangs[0], 22)
    siblings = s.fill(fangs[1], 1) + s.fill(fangs[2], 1)
    pre = s.deny(fangs[0])
    support.expect(lab, 'b', 24)
    late_old = s.open('c', transport)
    grandfathered = s.fill(late_old, 63)
    late_new = s.open('c', other)
    pair = provider_pair(s, 'd', 'e', transport)
    primary = s.fill(pair, 22)
    fallback = s.fill(pair, 22)
    require(s.targets['d'].hits == s.targets['e'].hits == 22, 'provider placement')
    independent = s.open('f', 'tcp')
    controls = s.fill(independent, 1) + siblings + owner[-2:]
    half_fang = s.open('g', transport)
    s.open('g', other)
    half = s.fill(half_fang, 32)
    require(all(conn.recv(1) == b'' for conn in half), 'half-close downstream EOF missing')
    half_count = 32 if transport == 'quic' else 0
    support.expect(lab, 'g', half_count)
    if half_count:
        until(lab, lambda: len(s.targets['g'].connections) == 32, '32 retained target uploads')
    else:
        until(lab, lambda: not s.targets['g'].connections, 'TCP target-first sessions drain')
    with s.targets['g'].lock:
        original_half_targets = set(s.targets['g'].connections)
    result = s.wait(controls, phase)
    require(idle.states(owner) == ['open'] * 22, '22-slot clients ended before demand')
    # A payload after the idle interval makes this otherwise-old session recent.
    support.pulse(lab, [owner[1]])
    new = s.fill(fangs[0], 3)
    survivors = replacement(lab, owner, 3)
    require(all(conn in survivors for conn in (owner[1], *owner[-2:])), 'active/recent victim')
    support.pulse(lab, survivors + new + siblings)
    support.expect(lab, 'b', 24)
    # One free total slot belongs to the late Fang. A single old release could
    # not fit the grandfathered Fang's current 32 share, so it must not evict.
    late = s.fill(late_new, 1)
    s.deny(late_new)
    s.deny(late_old)
    require(idle.states(grandfathered) == ['open'] * 63, 'forced over-share rebalance')
    support.pulse(lab, grandfathered + late)
    closed = support.close_fang(lab, late_old, grandfathered)
    late.extend(s.fill(late_new, 63))
    support.expect(lab, 'c', 64)
    support.pulse(lab, late)
    # One peer victim is spent at D before its authenticated target refuses.
    # E is also idle/full, but cannot consume a second victim for this client.
    before = s.targets['e'].hits
    stop_listener(s.targets['d'])
    denied = s.deny(pair, count=1)
    primary_live = replacement(lab, primary, 1)
    require(idle.states(fallback) == ['open'] * 22 and s.targets['e'].hits == before,
            'second provider consumed another victim or payload')
    support.pulse(lab, primary_live + fallback)
    support.expect(lab, 'd', 21)
    support.expect(lab, 'e', 22)
    with s.targets['g'].lock:
        require(s.targets['g'].connections == original_half_targets,
                'half-close upload ended without demand')
    half_new = s.fill(half_fang, 1)
    require(half_new[0].recv(1) == b'', 'new half-close EOF missing')
    if half_count:
        def one_half_replaced():
            with s.targets['g'].lock:
                current = s.targets['g'].connections
                return (len(current & original_half_targets) == 31 and
                        len(current - original_half_targets) == 1)
        until(lab, one_half_replaced, 'exactly one original half-close upload reclaimed')
        # A previously consumed FIN stays EOF at the client even when its
        # retained upload is cancelled. Target identities measure release.
        observed = idle.states(half)
        require(observed == ['eof'] * 32, f'half-close client EOF changed: {observed}')
        support.expect(lab, 'g', 32)
    else:
        observed = idle.states(half)
        require(observed == ['eof'] * 32, observed)
        support.expect(lab, 'g', 0)
    result.update(remainder_share=22, mixed_transports=[transport, other, transport],
                  before_idle_probes=pre, remainder_old_resets=3,
                  active_protected=2, recent_protected=1, siblings_protected=2,
                  grandfathered_preserved=63, grandfathered_close_resets=closed,
                  after_membership_shrink=64, provider_budget_denial=denied,
                  peer_first_victims=1, second_provider_victims=0,
                  half_close_retained=half_count, half_close_old_handlers_released=1 if half_count else 0,
                  half_close_states=dict(collections.Counter(observed)))
    result.update(s.finish(independent, controls))
    return result


def routed(s, mode, hop, repetition, phase):
    lab = s.lab
    if mode == 'relay':
        for dest in ('b', 'c'):
            lab.ctl('a', 'route', 'set', 'wolf-' + dest, '--via', 'wolf-d', '--hop', hop)
            lab.ctl('d', 'target', 'allow', 'wolf-a', lab.address(dest + '_tcp'))
        order = ('c', 'b') if repetition % 2 == 0 else ('b', 'c')
        fangs = [s.open(dest, 'tcp') for dest in order]
        owner_dest = order[0]
    else:
        order = ('d', 'c') if repetition % 2 == 0 else ('c', 'd')
        fangs = []
        for relay in order:
            lab.ctl('a', 'route', 'set', 'wolf-b', '--via', 'wolf-' + relay, '--hop', hop)
            lab.ctl(relay, 'target', 'allow', 'wolf-a', lab.address('b_tcp'))
            fangs.append(s.open('b', 'tcp'))
        shown = {r['id']: r['transport'] for r in lab.fang_list()}
        require(all(f'via wolf-{relay}' in shown[fang['id']]
                    for fang, relay in zip(fangs, order)), shown)
        owner_dest = 'b'
    owner, sibling = s.fill(fangs[0], 32), s.fill(fangs[1], 32)
    for wolf in ('b', 'c', 'd'):
        support.expect(lab, wolf, 64 if wolf == ('d' if mode == 'relay' else 'b') else 32)
    pre = s.deny(fangs[0])
    independent = s.open('e', 'tcp')
    controls = s.fill(independent, 1)
    result = s.wait(controls, phase)
    before = s.targets[owner_dest].hits
    new = s.fill(fangs[0], 3)
    live = replacement(lab, owner, 3)
    require(s.targets[owner_dest].hits - before == 3, 'routed newcomer payload count')
    # Hold successes so concurrent probes exert real capacity pressure.
    concurrent_new = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        probes = list(pool.map(lambda _: s.probe(fangs[0], concurrent_new), range(12)))
    require(all(r['outcome'] in ('echo', 'reset') for r in probes), probes)
    successes = sum(r['outcome'] == 'echo' for r in probes)
    observed = idle.states(live)
    victims = observed.count('reset')
    require(1 <= successes <= victims <= 12 and observed.count('open') == len(live) - victims,
            f'concurrent victim/admission bounds: {successes}, {observed}')
    live = [conn for conn, state in zip(live, observed) if state == 'open'] + new + concurrent_new
    require(idle.states(sibling) == ['open'] * 32, 'routed sibling evicted')
    # Races may deny after their own victim was released. Refill free capacity,
    # then protect every live session before the active-pressure storm.
    live.extend(s.fill(fangs[0], 32 - len(live)))
    support.pulse(lab, live + sibling + controls)
    for wolf in ('b', 'c', 'd'):
        support.expect(lab, wolf, 64 if wolf == ('d' if mode == 'relay' else 'b') else 32)
    hits, lines = s.targets[owner_dest].hits, len(lab.a_log().splitlines())
    health = next(r for r in lab.fang_list() if r['id'] == fangs[0]['id'])['sessions_failed']
    began = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
        storm = list(pool.map(lambda _: s.probe(fangs[0]), range(600)))
    elapsed = time.monotonic() - began
    entry = next(r for r in lab.fang_list() if r['id'] == fangs[0]['id'])
    delta = entry['sessions_failed'] - health
    log_lines = len(lab.a_log().splitlines()) - lines
    require(all(r['outcome'] == 'reset' for r in storm) and delta == 600 and
            entry['last_failure']['class'] == 'dial', 'active storm outcome/health')
    require(s.targets[owner_dest].hits == hits, 'active storm reached target')
    require(log_lines <= 2 + int(elapsed / 10), f'unbounded denial logging: {log_lines}')
    support.pulse(lab, live + sibling + controls)
    result.update(order=list(order), before_idle_probes=pre, sequential_old_resets=3,
                  concurrent_clients=probes, concurrency=6, concurrent_victims=victims,
                  concurrent_echoes=successes, sibling_preserved=32,
                  storm_clients=600, storm_concurrency=16, storm_resets=600,
                  storm_failure_delta=delta, storm_payload_growth=0,
                  storm_log_lines=log_lines, storm_seconds=round(elapsed, 3))
    result.update(s.finish(independent, controls))
    return result


def source_case(s, transport, repetition, phase):
    lab = s.lab
    a, b, c = SOURCES
    fangs = [s.open('b', transport, True) for _ in range(4)]
    owner = s.fill(fangs[0], 16, a)
    siblings = sum((s.fill(fang, 1, b) for fang in fangs[1:]), [])
    pre = s.deny(fangs[0], a) + s.deny(fangs[0], b)
    pair = provider_pair(s, 'c', 'd', transport, True)
    # One free primary peer slot ensures its target attempt is reached even
    # if source cancellation has not yet released the old reservation.
    primary = s.fill(pair, 16, a) + s.fill(pair, 6, b)
    fallback = s.fill(pair, 10, b) + s.fill(pair, 12, c)
    require(s.targets['c'].hits == s.targets['d'].hits == 22, 'source provider placement')
    primary.pop().close()
    support.expect(lab, 'c', 21)
    independent = s.open('e', 'tcp')
    controls = s.fill(independent, 1) + siblings
    result = s.wait(controls, phase)
    phase('source-share', {})
    same_source = []
    first = s.probe(fangs[0], same_source, a)
    require(first['outcome'] in ('echo', 'reset'), first)
    # Source admission marks a victim without waiting for peer cleanup when
    # local Fang slots are free. The contract permits that newcomer to deny.
    support.expect(lab, 'b', 18 + len(same_source))
    survivors = replacement(lab, owner, 1)
    if first['outcome'] == 'reset':
        same_source.extend(s.fill(fangs[0], 1, a))
        require(idle.states(survivors) == ['open'] * 15,
                'release recovery consumed another victim')
    other_source = s.fill(fangs[0], 1, b)
    survivors = replacement(lab, survivors, 1)
    support.pulse(lab, survivors + same_source + other_source + controls)
    # Refreshed own clients and active sibling clients are protected.
    s.deny(fangs[0], a)
    s.deny(fangs[0], c)
    require(idle.states(survivors + same_source + other_source) == ['open'] * 16,
            'recent source clients evicted')
    support.expect(lab, 'b', 19)
    before = s.targets['d'].hits
    stop_listener(s.targets['c'])
    phase('source-provider-budget', {})
    denial = s.deny(pair, a, count=1)
    support.expect(lab, 'c', 20)
    primary_live = replacement(lab, primary, 1)
    require(idle.states(fallback) == ['open'] * 22 and s.targets['d'].hits == before,
            'source-first newcomer consumed a second peer victim')
    support.pulse(lab, primary_live + fallback)
    support.expect(lab, 'c', 20)
    support.expect(lab, 'd', 22)
    result.update(source_addresses=list(SOURCES), bind=HOST, share=16,
                  before_idle_probes=pre, same_source_evictions=1, peer_only_evictions=1,
                  source_first_probe=first, source_release_recovery=first['outcome'] == 'reset',
                  siblings_protected=3, recent_protected=16, source_first_victims=1,
                  second_provider_victims=0, provider_budget_denial=denial)
    result.update(s.finish(independent, controls))
    return result


def enter_namespace():
    """Established Stage43 private-root scaffold, re-executing this fixture."""
    if os.environ.get('WWP_STAGE54_NETNS') != '1':
        argv = ['unshare', '-rmn', '--fork', '--kill-child', '--', sys.executable,
                '-B', str(pathlib.Path(__file__).resolve()), *sys.argv[1:]]
        os.execvpe('unshare', argv, dict(os.environ, WWP_STAGE54_NETNS='1'))
    def sh(*argv):
        subprocess.run([str(a) for a in argv], check=True)
    root = pathlib.Path('/mnt')
    sh('mount', '--make-rprivate', '/')
    sh('mount', '-t', 'tmpfs', '-o', 'size=512m,mode=755', 'none', root)
    for name in ('usr', 'etc', 'bin', 'sbin', 'lib', 'lib32', 'lib64', 'libx32',
                 'opt', 'var', 'home', 'dev', 'proc'):
        host = pathlib.Path('/', name)
        if not host.exists():
            continue
        if host.is_symlink():
            (root / name).symlink_to(os.readlink(host))
            continue
        (root / name).mkdir()
        sh('mount', '--rbind', host, root / name)
        if name not in ('home', 'dev', 'proc'):
            sh('mount', '-o', 'remount,bind,ro', root / name)
    (root / 'tmp').mkdir()
    sh('mount', '-t', 'tmpfs', '-o', 'size=512m,mode=1777', 'none', root / 'tmp')
    os.chroot(root)
    os.chdir(idle.ROOT)
    sh('ip', 'link', 'set', 'lo', 'up')
    for address in (HOST, *SOURCES):
        sh('ip', 'addr', 'add', f'{address}/32', 'dev', 'lo')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('cases', nargs='*')
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--out', type=pathlib.Path, required=True)
    args = parser.parse_args()
    cases = args.cases or CASES
    require(args.rounds > 0 and all(c in CASES for c in cases), 'invalid rounds/cases')
    if any(c.startswith('source-') for c in cases):
        enter_namespace()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open('x') as handle:
        handle.write('{}\n')
    os.umask(0o077)
    source = idle.inputs()
    source[str(pathlib.Path(__file__).resolve().relative_to(idle.ROOT))] = idle.digest(pathlib.Path(__file__))
    document = {'status': 'RUNNING', 'source_inputs': source, 'rows': [],
                'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=idle.ROOT,
                                                  text=True).strip()}
    for repetition in range(1, args.rounds + 1):
        for name in cases:
            idle.source_guard(source)
            mode, transport = name.split('-')
            lab = support.refresh.dial.DialLab()
            wolves = ('b', 'c', 'd', 'e', 'f', 'g') if mode == 'boundary' else ('b', 'c', 'd', 'e')
            lab.WOLVES = ('a', *wolves)
            row, s = {'case': name, 'round': repetition}, None
            def phase(label, detail):
                document['running'] = {**row, 'phase': label, **detail}
                idle.checkpoint(args.out, document)
                print(json.dumps(document['running']), flush=True)
            try:
                # Assign before setup so even partial setup failures close targets.
                s = Scenario.__new__(Scenario)
                s.connections, s.targets = [], {}
                s.__init__(lab, wolves, half=mode == 'boundary')
                if mode == 'boundary':
                    row.update(boundary(s, transport, repetition, phase))
                elif mode == 'source':
                    row.update(source_case(s, transport, repetition, phase))
                else:
                    row.update(routed(s, mode, transport, repetition, phase))
                idle.source_guard(source)
                row['status'] = 'PASS'
            except Exception as exc:
                row.update(status='FAIL', error=f'{type(exc).__name__}: {exc}',
                           failure_logs={p.name: p.read_text(errors='replace')[-2000:]
                                         for p in lab.base.glob('*.log')})
            finally:
                row['daemon_sha256'] = getattr(lab, 'binary_hash', None)
                try:
                    if s is not None:
                        s.close()
                    lab.cleanup()
                    require(not lab.base.exists() and all(p.poll() is not None for p in lab.processes),
                            'private fixture directory or owned process remains')
                    row['cleanup'] = 'PASS'
                except Exception as exc:
                    row.update(status='FAIL', cleanup_error=str(exc))
                    lab.cleanup()
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
