#!/usr/bin/env python3
"""Stage44 M1: IPv6 reproduction (disposable lab, namespace sandbox only).

Runs in its own user, mount and network namespace (`unshare -rmn`) with a
private root and a private 512 MB /tmp, as tools/stage43/fairness.py does.
Unique-local addresses on `lo` stand in for IPv6 hosts; no host interface,
no WAN and no live wolf is touched. Each scenario runs in a fresh lab built
on the acceptance lab (tests/integration/run.py, ServiceGate), with the
Stage42 50 MB log guard.

  fang:<kind>      I1/I2. A at fd43::1 (receiver), B at fd43::2. B opens a
                   Fang of <kind> (quic, tcp, tcp-plain) bound to [::1] to an
                   echo target on [fd43::1], granted by A's Target Policy.
                   Records the open, an echo and a 1 MiB full-duplex transfer.
  discovery:<t>    I3. B advertises its IPv6 addresses; A refreshes over <t>.
  sync:<t>         I3. A and B share a genesis; A applies one admission; B
                   syncs from A over <t>.
  routed:<t>       I4. A, relay B and destination C on IPv6; a routed Fang
                   whose hop to B is <t> (quic or tcp).
  dualstack        I5. A listens on [::] (TCP and QUIC); B dials it over IPv6
                   and C over IPv4, each over QUIC and TCP.
  share:<kind>     I6. B's Fang is bound to [fd43::2] (non-loopback). Four
                   flood sources in fd43::/64 each open connections until
                   refused; a client in another /64 then connects.
  cli              I7. `service connect --bind ::1` and `--bind [::1]`.

Prints one JSON line per scenario; writes all to --out.
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
sys.path.insert(0, str(ROOT / 'tests/integration'))
sys.path.insert(0, str(ROOT / 'tools/stage42'))
sys.path.insert(0, str(ROOT / 'tools/stage43'))
sys.dont_write_bytecode = True
from run import require, write_json  # noqa: E402
from service_gate import ServiceGate  # noqa: E402
from fang_flood import LOG_CAP, LogCap  # noqa: E402
from fairness import fill  # noqa: E402

HOSTS = {'a': 'fd43::1', 'b': 'fd43::2', 'c': 'fd43::3'}
FLOOD_SOURCES = ['fd43::10', 'fd43::11', 'fd43::12', 'fd43::13']
LEGIT_SOURCE = 'fd43:0:0:1::2'  # another /64
ADDRESSES = [*HOSTS.values(), *FLOOD_SOURCES, LEGIT_SOURCE]


def bracket(host):
    return f'[{host}]' if ':' in host else host


class Echo(socketserver.BaseRequestHandler):
    """Echo with no idle timeout."""
    def handle(self):
        while data := self.request.recv(65536):
            self.request.sendall(data)


class Echo6(socketserver.ThreadingTCPServer):
    address_family = socket.AF_INET6
    daemon_threads = True
    allow_reuse_address = True


def echo_server(host):
    server = Echo6((host, 0), Echo)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f'[{host}]:{server.server_address[1]}'


def echo_ok(host, port, timeout=5, source=None):
    try:
        with socket.create_connection((host, port), timeout=timeout,
                                      source_address=(source, 0) if source else None) as s:
            s.settimeout(timeout)
            payload = os.urandom(1024)
            s.sendall(payload)
            got = b''
            while len(got) < len(payload):
                chunk = s.recv(65536)
                if not chunk:
                    return False
                got += chunk
            return got == payload
    except OSError:
        return False


def duplex(host, port, total=1 << 20):
    block = hashlib.sha256(b'stage44').digest() * 2048
    sent, received, count = hashlib.sha256(), hashlib.sha256(), 0
    try:
        with socket.create_connection((host, port), timeout=20) as conn:
            def send():
                try:
                    for _ in range(total // len(block)):
                        conn.sendall(block)
                        sent.update(block)
                    conn.shutdown(socket.SHUT_WR)
                except OSError:
                    pass
            sender = threading.Thread(target=send)
            sender.start()
            while data := conn.recv(65536):
                received.update(data)
                count += len(data)
            sender.join()
    except OSError as exc:
        return {'ok': False, 'bytes': count, 'error': type(exc).__name__}
    return {'ok': count == total and sent.digest() == received.digest(), 'bytes': count}


class V6Lab(ServiceGate):
    """The acceptance lab with each wolf on its own IPv6 address."""
    WOLVES = ('a', 'b')
    listen_host = {}  # wolf -> listen host override (dualstack)

    def host(self, key):
        wolf = key.split('_')[0]
        if key.endswith(('_tcp', '_quic')) and wolf in self.listen_host:
            return self.listen_host[wolf]
        if key.endswith(('_tcp', '_quic')):
            return HOSTS[wolf]
        return self.local_hosts.get(key, '::1')

    @property
    def local_hosts(self):
        return getattr(self, '_local_hosts', {})

    def reserve(self, key, udp=False):
        sock = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM if udp else socket.SOCK_STREAM)
        sock.bind((self.host(key), 0))
        self.ports[key] = sock.getsockname()[1]
        self.reservations[key] = sock

    def address(self, key):
        return f'{bracket(self.host(key))}:{self.ports[key]}'

    def dial_host(self, key):
        host = self.host(key)
        return '::1' if host == '::' else host

    def listener(self, key):
        with socket.create_connection((self.dial_host(key), self.ports[key]), timeout=1):
            pass

    def reserve_wolf_ports(self):
        for wolf in self.WOLVES:
            for _ in range(128):
                self.reserve(wolf + '_tcp')
                udp = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
                try:
                    udp.bind((self.host(wolf + '_tcp'), self.ports[wolf + '_tcp']))
                except OSError:
                    udp.close()
                    self.release(wolf + '_tcp')
                    continue
                self.ports[wolf + '_quic'] = self.ports[wolf + '_tcp']
                self.reservations[wolf + '_quic'] = udp
                break
            else:
                raise RuntimeError('cannot reserve paired isolated TCP/UDP port')

    def ctl_any(self, wolf, *args):
        """Run werewolfctl; return exit, parsed JSON (if any) and stderr."""
        argv = [str(self.control_binary), '--json', '--socket', str(self.base / f'{wolf}.sock'),
                *[str(a) for a in args]]
        result = subprocess.run(argv, cwd=self.base, env=self.env, capture_output=True,
                                text=True, timeout=60)
        with (self.base / 'cli.jsonl').open('a') as log:
            log.write(json.dumps({'wolf': wolf, 'argv': argv[1:], 'exit': result.returncode,
                                  'stdout': result.stdout, 'stderr': result.stderr}) + '\n')
        lines = result.stdout.strip().splitlines()
        try:
            body = json.loads(lines[-1]) if lines else None
        except ValueError:
            body = lines[-1]
        return {'exit': result.returncode, 'body': body, 'stderr': result.stderr.strip()[-400:]}

    def guard(self):
        big = {p.name: p.stat().st_size for p in self.base.glob('*.log')
               if p.stat().st_size > LOG_CAP}
        if big:
            self.stop_wolves()
            raise LogCap(f'log cap exceeded, daemons stopped: {big}')

    def wait(self, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.guard()
            time.sleep(min(0.5, max(0, deadline - time.monotonic())))
        self.guard()

    def log_lines(self, wolf, count=12):
        """The daemon's last log lines (the lab's /tmp is private and goes
        with the namespace, so the evidence is kept in the result)."""
        lines = []
        for path in sorted(self.base.glob(f'*-{wolf}-*.log')):
            lines += [line[:300] for line in path.read_text(errors='replace').splitlines()]
        return lines[-count:]

    def boot(self, pack):
        """Start the wolves, init Pelts, and add the given Pack entries.

        pack: {wolf: [(peer_wolf, scheme, address)]}; address is a port key
        (resolved once ports are reserved) or a callable.
        """
        self.prepare_binaries()
        self.reserve_wolf_ports()
        for wolf in self.WOLVES:
            (self.base / wolf).mkdir(mode=0o700, exist_ok=True)
            write_json(self.base / wolf / 'target_policy.json',
                       {'mode': 'deny-by-default', 'peers': {}})
        self.start_wolves()
        self.keys = {}
        for wolf in self.WOLVES:
            self.ctl(wolf, 'pelt', 'init')
            self.keys[wolf] = self.ctl(wolf, 'pelt', 'show')['result']['public_key_b64']
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'), f'{wolf} TCP')
        for wolf, entries in pack.items():
            for peer, scheme, address in entries:
                address = address() if callable(address) else self.address(address)
                self.ctl(wolf, 'pack', 'add', f'wolf-{peer}', self.keys[peer],
                         f'{scheme}://{address}')

    def connect(self, wolf, name, key, transport, bind='[::1]'):
        self.release(key)
        opened = self.ctl_any(wolf, 'service', 'connect', name, '--local-port', self.ports[key],
                              '--bind', bind, '--transport', transport)
        listening = False
        if opened['exit'] == 0:
            try:
                self.wait_for(lambda: self.listener(key), f'{key} listener')
                listening = True
            except RuntimeError:
                pass
        return opened, listening


def scenario_fang(lab, kind):
    lab.boot({'a': [('b', 'quic', 'b_quic')],
              'b': [('a', 'quic', 'a_quic')]})
    server, target = echo_server(HOSTS['a'])
    lab.ctl('a', 'target', 'allow', 'wolf-b', target)
    lab.ctl('b', 'service', 'add', 'echo', 'wolf-a', target, 'tcp')
    lab.reserve('fang')
    opened, listening = lab.connect('b', 'echo', 'fang', kind)
    result = {'scenario': f'fang:{kind}', 'peer': lab.address('a_quic'), 'target': target,
              'fang_local': lab.address('fang'), 'open_exit': opened['exit'],
              'open': opened['body'] if opened['exit'] else opened['body'].get('result'),
              'open_stderr': opened['stderr'], 'listening': listening}
    if listening:
        port = lab.ports['fang']
        result.update(echo=echo_ok('::1', port), duplex_1mib=duplex('::1', port))
    lab.wait(1)
    result.update(b_log=lab.log_lines('b'), a_log=lab.log_lines('a'),
                  daemons_alive=all(w.poll() is None for w in lab.wolves))
    server.shutdown()
    return result


def scenario_discovery(lab, transport):
    lab.boot({'a': [('b', 'quic', 'b_quic')],
              'b': [('a', 'quic', 'a_quic')]})
    advertised = [f'quic://{lab.address("b_quic")}', f'tcp://{lab.address("b_tcp")}']
    adv = lab.ctl_any('b', 'discover', 'advertise', *advertised)
    refreshed = lab.ctl_any('a', 'discover', 'refresh', '--transport', transport)
    contacts = lab.ctl_any('a', 'discover', 'contacts')
    queried = (refreshed['body'] or {}).get('result', {}).get('queried') \
        if isinstance(refreshed['body'], dict) else refreshed['body']
    cached = contacts['body'].get('result') if isinstance(contacts['body'], dict) else None
    return {'scenario': f'discovery:{transport}', 'advertise_exit': adv['exit'],
            'advertise_stderr': adv['stderr'], 'refresh_exit': refreshed['exit'],
            'queried': queried, 'refresh_stderr': refreshed['stderr'],
            'cached_addresses': [c.get('addresses') for c in cached or []],
            'a_log': lab.log_lines('a'),
            'daemons_alive': all(w.poll() is None for w in lab.wolves)}


def scenario_sync(lab, transport):
    lab.boot({'a': [('b', 'quic', 'b_quic')],
              'b': [('a', 'quic', 'a_quic')]})
    for wolf in lab.WOLVES:
        lab.ctl(wolf, 'state', 'manifest-migrate')
    gdir = lab.base / 'governance'
    gdir.mkdir(mode=0o700)

    def gov(*args):
        return lab.ctl('a', 'governance', *args)

    def statement(name, draft_args):
        draft = gdir / f'{name}.draft'
        gov('draft', *draft_args, '--out', draft)
        parts = []
        for i in range(7):
            part = gdir / f'{name}.{i}.part'
            gov('sign', draft, gdir / f'k{i}.key', '--index', i, '--out', part)
            parts.append(part)
        out = gdir / f'{name}.wwgov'
        gov('combine', draft, *parts, '--out', out)
        return out

    publics = [gov('keygen', gdir / f'k{i}.key')['result']['public_key_b64'] for i in range(10)]
    genesis = statement('genesis', ['genesis', *[x for p in publics for x in ('--key', p)]])
    genesis_hash = gov('inspect', genesis)['result']['statement']['hash']
    for wolf in lab.WOLVES:
        lab.ctl(wolf, 'governance', 'apply', genesis, '--expect-hash', genesis_hash)
    member = gov('keygen', gdir / 'member.key')['result']['public_key_b64']
    admit = statement('admit', ['admit', '--after', genesis, '--member', member])
    lab.ctl('a', 'governance', 'apply', admit)
    synced = lab.ctl_any('b', 'governance', 'sync', '--transport', transport)
    body = synced['body'].get('result') if isinstance(synced['body'], dict) else synced['body']
    head = lab.ctl('b', 'governance', 'status')['result']['sequence']
    return {'scenario': f'sync:{transport}', 'sync_exit': synced['exit'],
            'applied': [s['sequence'] for s in (body or {}).get('applied', [])]
            if isinstance(body, dict) else None,
            'peers': body.get('peers') if isinstance(body, dict) else body,
            'sync_stderr': synced['stderr'], 'b_head': head, 'b_log': lab.log_lines('b'),
            'daemons_alive': all(w.poll() is None for w in lab.wolves)}


def scenario_routed(lab, hop):
    lab.WOLVES = ('a', 'b', 'c')
    lab.boot({'a': [('b', 'quic', 'b_quic'), ('c', 'tcp', 'c_tcp')],
              'b': [('a', 'quic', 'a_quic')],
              'c': [('a', 'quic', 'a_quic')]})
    server, target = echo_server(HOSTS['c'])
    lab.ctl('a', 'service', 'add', 'echo', 'wolf-c', target, 'tcp')
    lab.ctl('a', 'route', 'set', 'wolf-c', '--via', 'wolf-b', '--hop', hop)
    lab.ctl('b', 'target', 'allow', 'wolf-a', lab.address('c_tcp'))
    lab.ctl('c', 'target', 'allow', 'wolf-a', target)
    lab.reserve('fang')
    opened, listening = lab.connect('a', 'echo', 'fang', 'tcp')
    result = {'scenario': f'routed:{hop}', 'open_exit': opened['exit'],
              'open': opened['body'] if opened['exit'] else opened['body'].get('result'),
              'open_stderr': opened['stderr'], 'listening': listening}
    if listening:
        port = lab.ports['fang']
        result.update(echo=echo_ok('::1', port), duplex_1mib=duplex('::1', port))
    lab.wait(1)
    result.update(a_log=lab.log_lines('a'), b_log=lab.log_lines('b'),
                  daemons_alive=all(w.poll() is None for w in lab.wolves))
    server.shutdown()
    return result


def scenario_dualstack(lab):
    lab.WOLVES = ('a', 'b', 'c')
    lab.listen_host = {'a': '::', 'c': '127.0.0.1'}
    # C is an IPv4 host: its own listener on 127.0.0.1 (reserved over IPv6
    # needs an IPv4 socket).
    orig_reserve = V6Lab.reserve

    def reserve(key, udp=False):
        if lab.host(key) == '127.0.0.1':
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM if udp else socket.SOCK_STREAM)
            sock.bind(('127.0.0.1', 0))
            lab.ports[key] = sock.getsockname()[1]
            lab.reservations[key] = sock
        else:
            orig_reserve(lab, key, udp)
    lab.reserve = reserve
    v6_peer = f'[{HOSTS["a"]}]:{{port}}'
    lab.prepare_binaries()
    # Reserve C's pair on IPv4 by hand, then A and B on IPv6.
    lab.WOLVES = ('a', 'b')
    V6Lab.reserve_wolf_ports(lab)
    for _ in range(128):
        reserve('c_tcp')
        udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            udp.bind(('127.0.0.1', lab.ports['c_tcp']))
        except OSError:
            udp.close()
            lab.release('c_tcp')
            continue
        lab.ports['c_quic'] = lab.ports['c_tcp']
        lab.reservations['c_quic'] = udp
        break
    lab.WOLVES = ('a', 'b', 'c')
    lab.reserve_wolf_ports = lambda: None
    lab.prepare_binaries = lambda: None
    port = lab.ports['a_quic']
    lab.boot({'a': [('b', 'quic', 'b_quic'), ('c', 'quic', 'c_quic')],
              'b': [('a', 'quic', lambda: v6_peer.format(port=port))],
              'c': [('a', 'quic', lambda: f'127.0.0.1:{port}')]})
    server, target = echo_server(HOSTS['a'])
    for peer in ('b', 'c'):
        lab.ctl('a', 'target', 'allow', f'wolf-{peer}', target)
    result = {'scenario': 'dualstack', 'a_listen': lab.address('a_quic')}
    for wolf, family in (('b', 'ipv6'), ('c', 'ipv4')):
        lab.ctl(wolf, 'service', 'add', 'echo', 'wolf-a', target, 'tcp')
        for transport in ('quic', 'tcp'):
            key = f'{wolf}_{transport}_fang'
            lab._local_hosts = {**lab.local_hosts, key: '::1'}
            lab.reserve(key)
            opened, listening = lab.connect(wolf, 'echo', key, transport)
            result[f'{family}_{transport}'] = {
                'open_exit': opened['exit'], 'listening': listening,
                'echo': listening and echo_ok('::1', lab.ports[key])}
    lab.wait(1)
    result.update(a_log=lab.log_lines('a'),
                  daemons_alive=all(w.poll() is None for w in lab.wolves))
    server.shutdown()
    return result


def scenario_share(lab, kind):
    lab._local_hosts = {'fang': HOSTS['b']}
    lab.boot({'a': [('b', 'quic', 'b_quic')],
              'b': [('a', 'quic', 'a_quic')]})
    server, target = echo_server(HOSTS['a'])
    lab.ctl('a', 'target', 'allow', 'wolf-b', target)
    lab.ctl('b', 'service', 'add', 'echo', 'wolf-a', target, 'tcp')
    lab.reserve('fang')
    opened, listening = lab.connect('b', 'echo', 'fang', kind, bind=f'[{HOSTS["b"]}]')
    require(listening, f'share: Fang did not open: {opened}')
    host, port = HOSTS['b'], lab.ports['fang']
    result = {'scenario': f'share:{kind}', 'fang_local': lab.address('fang'),
              'legit_before': echo_ok(host, port, source=LEGIT_SOURCE)}
    held, socks = {}, []
    for source in FLOOD_SOURCES:
        got, _ = fill(lab, host, port, source)
        held[source] = len(got)
        socks += got
    result.update(flood_held=held, flood_total=len(socks),
                  legit_during=echo_ok(host, port, source=LEGIT_SOURCE))
    for s in socks:
        s.close()
    lab.wait(5)
    result.update(legit_after_release=echo_ok(host, port, 10, source=LEGIT_SOURCE),
                  daemons_alive=all(w.poll() is None for w in lab.wolves))
    server.shutdown()
    return result


def scenario_cli(lab):
    lab.boot({'a': [('b', 'quic', 'b_quic')],
              'b': [('a', 'quic', 'a_quic')]})
    server, target = echo_server(HOSTS['a'])
    lab.ctl('a', 'target', 'allow', 'wolf-b', target)
    lab.ctl('b', 'service', 'add', 'echo', 'wolf-a', target, 'tcp')
    result = {'scenario': 'cli'}
    for label, bind in (('bind_unbracketed', '::1'), ('bind_bracketed', '[::1]')):
        key = f'fang_{label}'
        lab.reserve(key)
        opened, listening = lab.connect('b', 'echo', key, 'tcp', bind=bind)
        result[label] = {'bind': bind, 'exit': opened['exit'], 'listening': listening,
                         'body': opened['body'], 'stderr': opened['stderr'],
                         'echo': listening and echo_ok('::1', lab.ports[key])}
    server.shutdown()
    return result


def sh(*argv):
    subprocess.run([str(a) for a in argv], check=True)


def enter_netns():
    """Re-exec inside a fresh user, mount and network namespace.

    As tools/stage43/fairness.py: a private root the namespace owns (the
    daemon refuses a home whose ancestors belong to an unmapped owner), the
    checkout's /home, the host's /proc and /dev, and a private 512 MB /tmp.
    """
    if os.environ.get('WWP_STAGE44_NETNS') != '1':
        env = dict(os.environ, WWP_STAGE44_NETNS='1')
        argv = ['unshare', '-rmn', '--fork', '--kill-child', '--',
                sys.executable, '-B', str(pathlib.Path(__file__).resolve()), *sys.argv[1:]]
        os.execvpe('unshare', argv, env)
    root = pathlib.Path('/mnt')
    sh('mount', '--make-rprivate', '/')
    sh('mount', '-t', 'tmpfs', '-o', 'size=512m,mode=755', 'none', root)
    for name in ('usr', 'etc', 'bin', 'sbin', 'lib', 'lib32', 'lib64', 'libx32', 'opt', 'var',
                 'home', 'dev', 'proc'):
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
    os.chdir(ROOT)
    sh('ip', 'link', 'set', 'lo', 'up')
    for address in ADDRESSES:
        sh('ip', '-6', 'addr', 'add', f'{address}/128', 'dev', 'lo', 'nodad')


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('scenarios', nargs='+', help='e.g. fang:quic sync:tcp dualstack')
    parser.add_argument('--out', required=True)
    parser.add_argument('--keep-temp', action='store_true')
    args = parser.parse_args()
    out = pathlib.Path(args.out).resolve()
    enter_netns()

    results = []
    for scenario in args.scenarios:
        mode, _, arg = scenario.partition(':')
        os.umask(0o077)
        lab = V6Lab(args.keep_temp)
        try:
            run = {'fang': scenario_fang, 'discovery': scenario_discovery,
                   'sync': scenario_sync, 'routed': scenario_routed,
                   'share': scenario_share}.get(mode)
            if run and arg:
                result = run(lab, arg)
            elif mode == 'dualstack':
                result = scenario_dualstack(lab)
            elif mode == 'cli':
                result = scenario_cli(lab)
            else:
                raise SystemExit(f'unknown scenario {scenario}')
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
