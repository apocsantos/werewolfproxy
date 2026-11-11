#!/usr/bin/env python3
"""Stage46 M1: dialling a hostname with several addresses (disposable lab).

Always runs in a sandbox: a fresh user, mount and network namespace with a
private root (as tools/stage45/visibility.py --sandbox-hosts), a private
/etc/hosts and /etc/gai.conf, and a `dummy` interface that silently drops
everything routed to 10.46.0.0/24. Nothing on the host changes. The lab is
the Stage45 one (tests/integration ServiceGate, fresh Dens, the checkout
build, the Stage42 50 MB log guard); every daemon listens on 127.0.0.1 only.

Each shape is a name whose addresses come back from getaddrinfo in this
order (the private gai.conf fixes it; the result records the order seen):

  fast     127.0.0.2 (nothing listens: TCP refused, UDP port unreachable),
           then 127.0.0.1
  silent   10.46.0.9 (dropped silently), then 127.0.0.1
  family   ::1 (nothing listens), then 127.0.0.1: the host's `localhost`
  dead     127.0.0.2, then 10.46.0.9: no address answers

Scenarios are <path>:<shape>:

  quic, tcp        A's direct Fang to B, whose Pack address on A is
                   <scheme>://<shape>.stage46:PORT
  tcp-plain        A's plain-TCP Fang whose service target is
                   <shape>.stage46:ECHO_PORT
  failover         providers B (Pack address by name, QUIC) then C (literal)
  routed-quic,     A's Fang to C routed via relay B, whose Pack address on A
  routed-tcp       is the name; hop QUIC or TCP
  discovery-quic,  A's `discover refresh` of B, B's address by name
  discovery-tcp
  sync-quic,       B's `governance sync` from A, A's address on B by name
  sync-tcp

Fang scenarios record, per client (one at a time): the outcome and the
seconds to it; A's log lines; A's `fang.list` entry. Discovery and sync
record the per-peer status and the seconds. Prints one JSON line per
scenario; writes all to --out.
"""

import argparse
import json
import os
import pathlib
import socket
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/integration'))
sys.path.insert(0, str(ROOT / 'tools/stage45'))
sys.dont_write_bytecode = True
from run import require  # noqa: E402
from visibility import LogCap, VisLab, attempt  # noqa: E402

SHAPES = {
    'fast': ['127.0.0.2', '127.0.0.1'],
    'silent': ['10.46.0.9', '127.0.0.1'],
    'family': ['::1', '127.0.0.1'],
    'dead': ['127.0.0.2', '10.46.0.9'],
}
# Any `precedence` line replaces glibc's whole default table: the RFC 6724
# defaults, plus the two "bad" IPv4 addresses ranked above other IPv4.
GAI_CONF = """precedence ::1/128 50
precedence ::/0 40
precedence 2002::/16 30
precedence ::/96 20
precedence ::ffff:127.0.0.2/128 60
precedence ::ffff:10.46.0.0/120 60
precedence ::ffff:0:0/96 35
"""


def name(shape):
    return f'{shape}.stage46'


def resolved(shape):
    try:
        infos = socket.getaddrinfo(name(shape), 1, type=socket.SOCK_STREAM)
    except OSError as exc:
        return str(exc)
    return [info[4][0] for info in infos]


class DialLab(VisLab):
    def measure_timed(self, count):
        before = len(self.a_log())
        outcomes = []
        for _ in range(count):
            self.guard()
            start = time.monotonic()
            outcome = attempt(self.ports['fang'])
            outcomes.append([outcome, round(time.monotonic() - start, 2)])
        time.sleep(1)
        lines = self.a_log()[before:].decode(errors='replace').splitlines()
        fangs = self.fang_list()
        return {'clients': outcomes,
                'echo': sum(1 for o, _ in outcomes if o == 'echo'),
                'a_log_lines': len(lines),
                'a_log_sample': [line[:240] for line in lines[:3]],
                'fang_list': fangs,
                'daemons_alive': all(w.poll() is None for w in self.wolves)}

    def pack_literal(self, wolf, peer, scheme='quic'):
        key = f'{peer}_{"quic" if scheme == "quic" else "tcp"}'
        return self.pack(wolf, peer, f'{scheme}://{self.address(key)}')

    def pack_name(self, wolf, peer, shape, scheme):
        # One port serves both transports (reserve_wolf_ports pairs them).
        return self.pack(wolf, peer, f'{scheme}://{name(shape)}:{self.ports[peer + "_quic"]}')


def open_and_measure(lab, transport, attempts, result):
    opened = lab.connect(transport)
    result['open_exit'] = opened['exit']
    result['open'] = (opened['body'].get('result') if opened['exit'] == 0
                      else {'body': opened['body'], 'stderr': opened['stderr']})
    if opened['exit'] == 0:
        result.update(lab.measure_timed(attempts))
    return result


def scenario_fang(lab, path, shape, attempts):
    lab.boot()
    result = {'scenario': f'{path}:{shape}', 'resolved': resolved(shape)}
    for wolf in ('b', 'c'):
        lab.pack_literal(wolf, 'a')
    if path in ('quic', 'tcp'):
        result['pack_add'] = lab.pack_name('a', 'b', shape, path)['exit']
        lab.ctl('a', 'service', 'add', 'echo', 'wolf-b', lab.echo, 'tcp')
        lab.ctl('b', 'target', 'allow', 'wolf-a', lab.echo)
        return open_and_measure(lab, path, attempts, result)
    if path == 'tcp-plain':
        lab.pack_literal('a', 'b')
        target = f'{name(shape)}:{lab.echo.rsplit(":", 1)[1]}'
        added = lab.ctl_any('a', 'service', 'add', 'echo', 'wolf-b', target, 'tcp')
        result['service_add'] = {'exit': added['exit'], 'stderr': added['stderr']}
        if added['exit'] != 0:
            return result
        return open_and_measure(lab, 'tcp-plain', attempts, result)
    if path == 'failover':
        result['pack_add'] = lab.pack_name('a', 'b', shape, 'quic')['exit']
        lab.pack_literal('a', 'c')
        lab.ctl('a', 'service', 'add', 'echo', 'wolf-b', lab.echo, 'tcp')
        lab.ctl('a', 'service', 'provider', 'add', 'echo', 'wolf-c', lab.echo)
        for wolf in ('b', 'c'):
            lab.ctl(wolf, 'target', 'allow', 'wolf-a', lab.echo)
        return open_and_measure(lab, 'quic', attempts, result)
    if path in ('routed-quic', 'routed-tcp'):
        hop = path.split('-')[1]
        result['pack_add'] = lab.pack_name('a', 'b', shape, hop)['exit']
        lab.pack_literal('a', 'c', 'tcp')
        lab.ctl('a', 'service', 'add', 'echo', 'wolf-c', lab.echo, 'tcp')
        lab.ctl('a', 'route', 'set', 'wolf-c', '--via', 'wolf-b', '--hop', hop)
        lab.ctl('b', 'target', 'allow', 'wolf-a', lab.address('c_tcp'))
        lab.ctl('c', 'target', 'allow', 'wolf-a', lab.echo)
        return open_and_measure(lab, 'tcp', attempts, result)
    raise SystemExit(f'unknown path {path}')


def scenario_discovery(lab, transport, shape):
    lab.boot()
    result = {'scenario': f'discovery-{transport}:{shape}', 'resolved': resolved(shape)}
    result['pack_add'] = lab.pack_name('a', 'b', shape, 'quic')['exit']
    lab.pack_literal('b', 'a')
    lab.ctl_any('b', 'discover', 'advertise', f'quic://{lab.address("b_quic")}',
                f'tcp://{lab.address("b_tcp")}')
    start = time.monotonic()
    refreshed = lab.ctl_any('a', 'discover', 'refresh', '--transport', transport)
    result['seconds'] = round(time.monotonic() - start, 2)
    body = refreshed['body']
    result['refresh_exit'] = refreshed['exit']
    result['queried'] = (body.get('result') or {}).get('queried') if isinstance(body, dict) \
        else body
    if refreshed['exit']:
        result['refresh_stderr'] = refreshed['stderr']
    result['daemons_alive'] = all(w.poll() is None for w in lab.wolves)
    return result


def scenario_sync(lab, transport, shape):
    lab.boot()
    result = {'scenario': f'sync-{transport}:{shape}', 'resolved': resolved(shape)}
    lab.pack_literal('a', 'b')
    result['pack_add'] = lab.pack_name('b', 'a', shape, 'quic')['exit']
    for wolf in ('a', 'b'):
        lab.ctl(wolf, 'state', 'manifest-migrate')
    gdir = lab.base / 'governance'
    gdir.mkdir(mode=0o700)

    def gov(*args):
        return lab.ctl('a', 'governance', *args)

    def statement(label, draft_args):
        draft = gdir / f'{label}.draft'
        gov('draft', *draft_args, '--out', draft)
        parts = []
        for i in range(7):
            part = gdir / f'{label}.{i}.part'
            gov('sign', draft, gdir / f'k{i}.key', '--index', i, '--out', part)
            parts.append(part)
        out = gdir / f'{label}.wwgov'
        gov('combine', draft, *parts, '--out', out)
        return out

    publics = [gov('keygen', gdir / f'k{i}.key')['result']['public_key_b64'] for i in range(10)]
    genesis = statement('genesis', ['genesis', *[x for p in publics for x in ('--key', p)]])
    genesis_hash = gov('inspect', genesis)['result']['statement']['hash']
    for wolf in ('a', 'b'):
        lab.ctl(wolf, 'governance', 'apply', genesis, '--expect-hash', genesis_hash)
    member = gov('keygen', gdir / 'member.key')['result']['public_key_b64']
    lab.ctl('a', 'governance', 'apply', statement('admit', ['admit', '--after', genesis,
                                                             '--member', member]))
    start = time.monotonic()
    synced = lab.ctl_any('b', 'governance', 'sync', '--transport', transport)
    result['seconds'] = round(time.monotonic() - start, 2)
    body = synced['body'].get('result') if isinstance(synced['body'], dict) else synced['body']
    result['sync_exit'] = synced['exit']
    result['peers'] = body.get('peers') if isinstance(body, dict) else body
    result['b_head'] = lab.ctl('b', 'governance', 'status')['result']['sequence']
    result['daemons_alive'] = all(w.poll() is None for w in lab.wolves)
    return result


def sh(*argv):
    subprocess.run([str(a) for a in argv], check=True)


def enter_sandbox():
    """Re-exec in a fresh user, mount and network namespace: a private root
    the namespace owns, the checkout's /home, the host's /proc and /dev, a
    private 512 MB /tmp, a private /etc/hosts and /etc/gai.conf, and the
    `dummy` drop route."""
    if os.environ.get('WWP_STAGE46_SANDBOX') != '1':
        env = dict(os.environ, WWP_STAGE46_SANDBOX='1')
        argv = ['unshare', '-rmn', '--fork', '--kill-child', '--',
                sys.executable, '-B', str(pathlib.Path(__file__).resolve()), *sys.argv[1:]]
        os.execvpe('unshare', argv, env)
    root = pathlib.Path('/mnt')
    sh('mount', '--make-rprivate', '/')
    sh('mount', '-t', 'tmpfs', '-o', 'size=512m,mode=755', 'none', root)
    for entry in ('usr', 'etc', 'bin', 'sbin', 'lib', 'lib32', 'lib64', 'libx32', 'opt', 'var',
                  'home', 'dev', 'proc'):
        host = pathlib.Path('/', entry)
        if not host.exists():
            continue
        if host.is_symlink():
            (root / entry).symlink_to(os.readlink(host))
            continue
        (root / entry).mkdir()
        sh('mount', '--rbind', host, root / entry)
        if entry not in ('home', 'dev', 'proc'):
            sh('mount', '-o', 'remount,bind,ro', root / entry)
    hosts = ['127.0.0.1\tlocalhost', '::1\tlocalhost ip6-localhost']
    hosts += [f'{address}\t{name(shape)}' for shape, addresses in SHAPES.items()
              for address in addresses]
    (root / 'hosts').write_text('\n'.join(hosts) + '\n')
    sh('mount', '--bind', root / 'hosts', root / 'etc/hosts')
    (root / 'gai.conf').write_text(GAI_CONF)
    if not (root / 'etc/gai.conf').exists():
        raise SystemExit('/etc/gai.conf missing on the host; cannot bind over it')
    sh('mount', '--bind', root / 'gai.conf', root / 'etc/gai.conf')
    (root / 'tmp').mkdir()
    sh('mount', '-t', 'tmpfs', '-o', 'size=512m,mode=1777', 'none', root / 'tmp')
    os.chroot(root)
    os.chdir(ROOT)
    sh('ip', 'link', 'set', 'lo', 'up')
    sh('ip', 'link', 'add', 'd46', 'type', 'dummy')
    sh('ip', 'link', 'set', 'd46', 'up')
    sh('ip', 'addr', 'add', '10.46.0.1/32', 'dev', 'd46')
    sh('ip', 'route', 'add', '10.46.0.0/24', 'dev', 'd46', 'src', '10.46.0.1')
    for shape, expected in SHAPES.items():
        require(resolved(shape) == expected,
                f'{name(shape)} resolves to {resolved(shape)}, expected {expected}')


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('scenarios', nargs='+', help='e.g. quic:family discovery-tcp:silent')
    parser.add_argument('--attempts', type=int, default=5)
    parser.add_argument('--out', required=True)
    parser.add_argument('--keep-temp', action='store_true')
    args = parser.parse_args()
    out = pathlib.Path(args.out).resolve()
    enter_sandbox()

    results = []
    for scenario in args.scenarios:
        path, _, shape = scenario.partition(':')
        if shape not in SHAPES:
            raise SystemExit(f'unknown shape in {scenario}')
        os.umask(0o077)
        lab = DialLab(args.keep_temp)
        try:
            if path.startswith('discovery-'):
                result = scenario_discovery(lab, path.split('-')[1], shape)
            elif path.startswith('sync-'):
                result = scenario_sync(lab, path.split('-')[1], shape)
            else:
                result = scenario_fang(lab, path, shape, args.attempts)
        except LogCap as exc:
            result = {'scenario': scenario, 'log_cap': str(exc)}
        except Exception as exc:  # record and continue with the next scenario
            result = {'scenario': scenario, 'error': f'{type(exc).__name__}: {exc}'}
        finally:
            result['daemon_sha256'] = getattr(lab, 'binary_hash', None)
            try:
                lab.cleanup()
            except Exception as exc:
                result['cleanup_error'] = str(exc)
        print(json.dumps(result), flush=True)
        results.append(result)
    out.write_text(json.dumps(results, indent=1) + '\n')


if __name__ == '__main__':
    main()
