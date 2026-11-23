#!/usr/bin/env python3
"""Stage55 secure/plain global-cap admission baseline and reclamation controls.

Fresh two-node checkout labs, actual 1024 global slots, 4096 task-owned
descriptor limit, real 65-second idle waits. Secure owner holds 32; plain
fillers hold 992, avoiding receiver/peer exhaustion. No installed nodes.
"""
import argparse
import collections
import concurrent.futures
import json
import os
import pathlib
import resource
import socket
import subprocess
import sys
import time

sys.dont_write_bytecode = True
import global_idle as base
from global_idle import idle, support, require, until, open_fang, attempt as plain_attempt, exchange
from regression import replacement

SOURCE_HOST = '10.54.0.1'
SOURCE_A, SOURCE_B, SOURCE_C = '10.54.0.2', '10.54.0.3', '10.54.0.4'


def attempt(port, held=None, source=None, host='127.0.0.1'):
    if source is None:
        return plain_attempt(port, held)
    conn = None
    started = time.monotonic()
    try:
        conn = socket.create_connection((host, port), timeout=5,
                                        source_address=(source, 0))
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


def enter_source_namespace():
    """Create a private loopback-only namespace for source-address controls."""
    if os.environ.get('WWP_STAGE55_NETNS') != '1':
        argv = ['unshare', '-rmn', '--fork', '--kill-child', '--', sys.executable,
                '-B', str(pathlib.Path(__file__).resolve()), *sys.argv[1:]]
        os.execvpe('unshare', argv, dict(os.environ, WWP_STAGE55_NETNS='1'))

    def sh(*argv):
        subprocess.run([str(value) for value in argv], check=True)

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
    os.chdir(base.ROOT)
    sh('ip', 'link', 'set', 'lo', 'up')
    for address in (SOURCE_HOST, SOURCE_A, SOURCE_B, SOURCE_C):
        sh('ip', 'addr', 'add', f'{address}/32', 'dev', 'lo')


def scenario(lab, transport, expectation, phase, concurrent_probes=0, routed_owner=False):
    target, connections = base.Echo(), []
    try:
        lab.boot()
        baseline = idle.fd_count(lab)
        peers = (('a', 'b'), ('b', 'a'))
        provider_owner = getattr(lab, 'provider_owner', False)
        source_owner = getattr(lab, 'source_owner', False)
        if routed_owner or provider_owner:
            peers += (('a', 'c'), ('c', 'a'))
        for wolf, peer in peers:
            require(lab.pack_literal(wolf, peer)['exit'] == 0, 'Pack setup')
        if routed_owner:
            lab.ctl('b', 'target', 'allow', 'wolf-a', target.address)
            lab.ctl('c', 'target', 'allow', 'wolf-a', target.address)
            lab.ctl('c', 'target', 'allow', 'wolf-a', lab.address('b_tcp'))
            lab.ctl('a', 'route', 'set', 'wolf-b', '--via', 'wolf-c', '--hop', transport)
            lab.ctl('a', 'service', 'add', 'global-owner', 'wolf-b', target.address, 'tcp')
            lab.ctl('a', 'service', 'add', 'global-fill', 'wolf-c', target.address, 'tcp')
        elif provider_owner:
            # Deliberately leave the primary destination unauthorized so the
            # provider path uses C at the real global ceiling. Existing
            # provider/commitment controls exercise the active-primary path.
            lab.ctl('c', 'target', 'allow', 'wolf-a', target.address)
            lab.ctl('a', 'service', 'add', 'global-owner', 'wolf-b', target.address, 'tcp')
            lab.ctl('a', 'service', 'provider', 'add', 'global-owner', 'wolf-c', target.address)
            lab.ctl('a', 'service', 'add', 'global-fill', 'wolf-c', target.address, 'tcp')
        else:
            lab.ctl('b', 'target', 'allow', 'wolf-a', target.address)
            lab.ctl('a', 'service', 'add', 'global', 'wolf-b', target.address, 'tcp')
        specs = [(transport, 32)] + [('tcp-plain', 32)] * 29 + [('tcp-plain', 64)]
        groups = []
        for i, (kind, n) in enumerate(specs):
            service = ('global-owner' if (routed_owner or provider_owner) and i == 0 else
                       'global-fill' if (routed_owner or provider_owner) else 'global')
            if source_owner and i == 0:
                lab.reserve('extra')
                port = lab.ports['extra']
                lab.release('extra')
                opened = lab.ctl_any('a', 'service', 'connect', service,
                                     '--local-port', port, '--transport', kind,
                                     '--bind', SOURCE_HOST)
                require(opened['exit'] == 0, opened)
                fang = {'port': port, 'id': opened['body']['result']['fang_id'],
                        'host': SOURCE_HOST}
            else:
                fang = open_fang(lab, service, kind)
            groups.append({'fang': fang, 'clients': [], 'slots': n})
        unused = open_fang(lab, 'global-fill' if (routed_owner or provider_owner) else 'global', 'tcp-plain')
        def guard():
            lab.guard()
            require(all(p.poll() is None for p in lab.wolves), 'fixture daemon exited')
            require(not target.snapshot()[2], target.snapshot()[2])
        phase('filling', {'global_slots': base.GLOBAL, 'secure_owner': transport})
        for group in groups:
            for index in range(group['slots']):
                guard()
                if source_owner and group is groups[0]:
                    source = SOURCE_A if index < 16 else SOURCE_B
                    result = attempt(group['fang']['port'], group['clients'], source,
                                     group['fang']['host'])
                    require(result['outcome'] == 'echo',
                            f'source fill failed at {index}: {result}')
                else:
                    idle.held(group['fang']['port'], group['clients'])
                connections.append(group['clients'][-1])
        require(len(connections) == base.GLOBAL, 'wrong production-cap workload')
        until(lab, lambda: target.snapshot()[:2] == (1024, 1024), '1024 target streams')
        authority = support.expect(lab, 'b', 32 if transport != 'tcp-plain' and not provider_owner else 0)
        if routed_owner:
            relay_authority = support.expect(lab, 'c', 32)
        if provider_owner:
            provider_authority_before = support.expect(lab, 'c', 32)
        until(lab, lambda: idle.fd_count(lab) < base.NOFILE, 'bounded saturated descriptors')
        expected_fds, steady = None, 0
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and steady < 4:
            current = idle.fd_count(lab)
            if current == expected_fds:
                steady += 1
            else:
                expected_fds, steady = current, 0
            time.sleep(.1)
        require(steady >= 4, f'descriptors did not stabilize: {expected_fds}, {steady}')
        require(expected_fds < base.NOFILE, 'descriptor exhaustion instead of global saturation')
        owner, control = groups[0], groups[-1]
        before = [attempt(owner['fang']['port'], source=SOURCE_A,
                          host=owner['fang'].get('host', '127.0.0.1'))
                  if source_owner else attempt(owner['fang']['port']),
                  attempt(control['fang']['port'])]
        require(all(r['outcome'] == 'reset' for r in before), before)
        began = time.monotonic()
        phase('idle-wait', {'seconds': idle.IDLE})
        while time.monotonic() - began < idle.IDLE:
            guard()
            time.sleep(min(1, max(0, idle.IDLE - (time.monotonic() - began))))
        elapsed = time.monotonic() - began
        require(elapsed >= idle.IDLE, 'shortened idle observation')
        require(idle.states(connections) == ['open'] * 1024, 'old session ended without demand')
        phase('post-idle', {})
        newcomers = []
        if concurrent_probes:
            def concurrent_attempt(_):
                held = []
                result = attempt(owner['fang']['port'], held)
                return result, held
            with concurrent.futures.ThreadPoolExecutor(max_workers=concurrent_probes) as pool:
                paired = list(pool.map(concurrent_attempt, range(concurrent_probes)))
            probes = [result for result, _ in paired]
            for _, held in paired:
                newcomers.extend(held)
        elif source_owner:
            probes = [attempt(owner['fang']['port'], newcomers, SOURCE_A,
                              owner['fang']['host']) for _ in range(3)]
        else:
            probes = [attempt(owner['fang']['port'], newcomers) for _ in range(3)]
        if concurrent_probes or source_owner:
            successes = [r for r in probes if r['outcome'] == 'echo']
            require(1 <= len(successes) <= len(probes), probes)
            states = ['open'] * len(owner['clients'])
            stable_since, last_states = time.monotonic(), None
            settle_deadline = stable_since + 5
            while time.monotonic() < settle_deadline:
                current = idle.states(owner['clients'])
                require(all(state in ('open', 'reset', 'eof') for state in current),
                        f'unexpected concurrent victim states: {current}')
                if current == last_states:
                    if time.monotonic() - stable_since >= .5:
                        states = current
                        break
                else:
                    stable_since, last_states = time.monotonic(), current
                time.sleep(.05)
            else:
                require(False, f'concurrent victim states did not settle: {last_states}')
            victims = states.count('reset') + states.count('eof')
            require(successes and len(successes) <= victims <= len(probes),
                    f'concurrent outcomes/victims: {len(successes)}, {victims}, {probes}')
            owner['clients'] = [conn for conn, state in zip(owner['clients'], states)
                                if state == 'open']
            refills = victims - len(successes)
            for _ in range(refills):
                refill = []
                outcome = attempt(owner['fang']['port'], refill,
                                  SOURCE_C if source_owner else None,
                                  owner['fang'].get('host', '127.0.0.1'))
                require(outcome['outcome'] == 'echo',
                        f'could not refill a slot after failed concurrent setup: {outcome}')
                newcomers.extend(refill)
        else:
            victims = 3 if expectation == 'reclaimed' else 0
            refills = 0
        connections.extend(newcomers)
        if concurrent_probes or source_owner:
            require(all(r['outcome'] in ('echo', 'reset', 'eof') for r in probes), probes)
        else:
            require(all(r['outcome'] == ('echo' if victims else 'reset') for r in probes), probes)
        if victims and not concurrent_probes and not source_owner:
            owner['clients'] = replacement(lab, owner['clients'], victims)
        elif not victims:
            require(idle.states(owner['clients']) == ['open'] * 32, 'baseline selected a victim')
        siblings = sum((g['clients'] for g in groups[1:]), [])
        require(idle.states(siblings) == ['open'] * 992, 'global pressure evicted another Fang')
        until(lab, lambda: target.snapshot()[:2] == (1024, 1024 + victims), 'owner target count')
        empty = [attempt(unused['port']) for _ in range(3)]
        require(all(r['outcome'] == 'reset' for r in empty), empty)
        require(idle.states(siblings) == ['open'] * 992, 'empty Fang stole capacity')
        for conn in owner['clients'] + newcomers:
            exchange(conn)
        protected = [attempt(owner['fang']['port'], source=SOURCE_A,
                             host=owner['fang'].get('host', '127.0.0.1'))
                     if source_owner else attempt(owner['fang']['port']) for _ in range(3)]
        require(all(r['outcome'] == 'reset' for r in protected), protected)

        positive = []
        require(attempt(control['fang']['port'], positive)['outcome'] == 'echo', 'local full control')
        connections.extend(positive)
        control['clients'] = replacement(lab, control['clients'], 1)
        until(lab, lambda: target.snapshot()[:2] == (1024, 1025 + victims), 'local positive target')
        groups[-2]['clients'].pop().close()
        until(lab, lambda: target.snapshot()[0] == 1023, 'released sibling target drained')
        until(lab, lambda: idle.fd_count(lab) == expected_fds - 2, 'released sibling descriptors')
        recovery = []
        recovered = attempt(owner['fang']['port'], recovery,
                            SOURCE_C if source_owner else None,
                            owner['fang'].get('host', '127.0.0.1'))
        connections.extend(recovery)
        require(recovered['outcome'] == 'echo', recovered)
        survivors = sum((g['clients'] for g in groups), []) + newcomers + positive + recovery
        require(len(survivors) == 1024, 'wrong survivor count')
        for conn in survivors:
            guard()
            exchange(conn)
        after_authority = support.expect(lab, 'b', 33 if transport != 'tcp-plain' and not provider_owner else 0)
        if routed_owner:
            relay_after_authority = support.expect(lab, 'c', 33)
        if provider_owner:
            provider_after_authority = support.expect(lab, 'c', 33)
        until(lab, lambda: target.snapshot()[:2] == (1024, 1026 + victims), 'recovery target count')
        cpu = support.idle_cpu(lab)
        phase('cleanup', {})
        for fang in lab.fang_list():
            lab.control('a', 'fang.close', {'fang_id': fang['id']})
        for conn in connections:
            conn.close()
        final = {w: support.expect(lab, w, 0) for w in
                 (('a', 'b', 'c') if routed_owner or provider_owner else ('a', 'b'))}
        until(lab, lambda: target.snapshot()[0] == 0, 'targets drained')
        until(lab, lambda: idle.fd_count(lab) <= baseline, 'initiator descriptors drained')
        guard()
        return {'transport': transport, 'routed_owner': routed_owner,
                'provider_owner': provider_owner,
                'source_owner': source_owner,
                'source_owner_sessions': 32 if source_owner else 0,
                'concurrent_probe_count': concurrent_probes,
                'concurrent_refills': refills,
                'concurrent_echoes': sum(r['outcome'] == 'echo' for r in probes)
                if concurrent_probes else None,
                'source_probe_outcomes': probes if source_owner else None,
                'route_authority_before': relay_authority if routed_owner else None,
                'route_authority_after_recovery': relay_after_authority if routed_owner else None,
                'provider_authority_before': provider_authority_before if provider_owner else None,
                'provider_authority_after_recovery': provider_after_authority if provider_owner else None,
                'idle_seconds': round(elapsed, 3),
                'global_slots': 1024, 'secure_owner_sessions': 0 if transport == 'tcp-plain' else 32,
                'initial_authority': authority, 'after_recovery_authority': after_authority,
                'before_idle_probes': before, 'post_idle_probes': probes,
                'global_victims': victims, 'sibling_sessions_preserved': 992,
                'empty_fang_denials': empty, 'recent_owner_denials': protected,
                'local_positive_admissions': 1, 'local_positive_victims': 1,
                'release_other_recovery': recovered, 'surviving_echoes': 1024,
                'target_payload_streams_total': target.snapshot()[1], 'target_streams_after': 0,
                'idle_cpu_pct': cpu, 'fds_before': baseline, 'fds_saturated': expected_fds,
                'fds_after': idle.fd_count(lab), 'final_authority': final, 'daemons_alive': True}
    finally:
        for conn in connections:
            conn.close()
        target.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('transports', nargs='*')
    parser.add_argument('--expect', choices=('baseline', 'reclaimed'), default='baseline')
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--concurrent-probes', type=int, default=0,
                        help='hold this many simultaneous owner probes after idle')
    parser.add_argument('--routed-owner', action='store_true',
                        help='route the secure owner through a second generated node')
    parser.add_argument('--provider-owner', action='store_true',
                        help='exercise provider fallback for the secure owner at global capacity')
    parser.add_argument('--source-owner', action='store_true',
                        help='saturate two source quotas on the secure owner at global capacity')
    parser.add_argument('--out', type=pathlib.Path, required=True)
    args = parser.parse_args()
    transports = args.transports or ('quic', 'tcp', 'tcp-plain')
    require(args.rounds > 0 and args.concurrent_probes >= 0 and
            all(t in ('quic', 'tcp', 'tcp-plain') for t in transports), 'invalid matrix')
    require(sum((args.routed_owner, args.provider_owner, args.source_owner)) <= 1,
            'choose one owner path control')
    require(not (args.routed_owner or args.provider_owner or args.source_owner) or
            all(t in ('quic', 'tcp') for t in transports),
            'routed/provider/source owner requires a secure transport')
    require(not args.source_owner or args.concurrent_probes == 0,
            'source owner is a sequential quota control')
    if args.source_owner:
        enter_source_namespace()
    _, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    require(hard == resource.RLIM_INFINITY or hard >= base.NOFILE, '4096 descriptor limit unavailable')
    resource.setrlimit(resource.RLIMIT_NOFILE, (base.NOFILE, base.NOFILE))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open('x') as handle:
        handle.write('{}\n')
    os.umask(0o077)
    source = idle.inputs()
    source[str(pathlib.Path(__file__).resolve().relative_to(base.ROOT))] = idle.digest(pathlib.Path(__file__))
    document = {'status': 'RUNNING', 'expectation': args.expect, 'source_inputs': source,
                'commit': subprocess.check_output(['git','rev-parse','HEAD'],cwd=base.ROOT,text=True).strip(),
                'rows': []}
    for repetition in range(1, args.rounds + 1):
        for transport in transports:
            idle.source_guard(source)
            lab = support.refresh.dial.DialLab()
            lab.WOLVES = ('a', 'b', 'c') if args.routed_owner or args.provider_owner else ('a', 'b')
            lab.provider_owner = args.provider_owner
            lab.source_owner = args.source_owner
            row = {'round': repetition, 'transport': transport}
            def phase(label, detail):
                document['running'] = {**row, 'phase': label, **detail}
                idle.checkpoint(args.out, document)
                print(json.dumps(document['running']), flush=True)
            try:
                row.update(scenario(lab, transport, args.expect, phase,
                                    args.concurrent_probes, args.routed_owner), status='PASS')
                idle.source_guard(source)
            except Exception as exc:
                row.update(status='FAIL', error=f'{type(exc).__name__}: {exc}',
                           failure_logs={p.name:p.read_text(errors='replace')[-2000:]
                                         for p in lab.base.glob('*.log')})
            finally:
                row['daemon_sha256'] = getattr(lab,'binary_hash',None)
                try:
                    lab.cleanup()
                    require(not lab.base.exists() and all(p.poll() is not None for p in lab.processes), 'fixture remains')
                    row['cleanup'] = 'PASS'
                except Exception as exc:
                    row.update(status='FAIL',cleanup_error=str(exc))
            document['rows'].append(row)
            document.pop('running',None)
            idle.checkpoint(args.out,document)
            print(json.dumps(row),flush=True)
            if row['status']=='FAIL':
                document['status']='FAIL'
                idle.checkpoint(args.out,document)
                return 1
    document['status']='PASS'
    idle.checkpoint(args.out,document)
    return 0


if __name__ == '__main__':
    sys.exit(main())
