#!/usr/bin/env python3
"""Stage45 M1: Fang failure visibility, reproduction (disposable lab).

Builds on the acceptance lab (tests/integration/run.py, ServiceGate): fresh
Dens, dynamic loopback ports, the checkout build, only its own process groups
are stopped, and the Stage42 50 MB log guard applies. No live wolf is touched.
Each scenario runs in a fresh lab. A is the initiator whose visibility is
measured; B and C are receivers.

  <cause>:<kind>   a Fang of <kind> whose sessions fail, ATTEMPTS clients:
     grant         the receiver does not grant the target (handshake refusal)
     dial          the peer's (or relay's, or providers') address is dead
     <kind>        quic, tcp, routed (QUIC hop B -> destination C), failover
                   (providers B then C), and for dial also tcp-plain (dead
                   target)
  flood:tcp-plain  V2: FLOOD connections to a plain-TCP Fang whose target is
                   dead; A's log growth
  hostname:<kind>  V4: B's Pack address on A is `<scheme>://localhost:PORT`;
                   B grants the target; quic and tcp

Records, per scenario: what A logged during the attempts (lines and bytes),
whether every client was reset, and A's `fang.list` entry before and after.
Prints one JSON line per scenario; writes all to --out.
"""

import argparse
import json
import os
import pathlib
import socket
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/integration'))
sys.dont_write_bytecode = True
from run import require, write_json  # noqa: E402
from service_gate import ServiceGate  # noqa: E402

LOG_CAP = 50 * 1024 * 1024  # the Stage42 guard: the lab lives on a small /tmp


class LogCap(RuntimeError):
    pass


def attempt(port, timeout=15):
    """One client: send 16 bytes, wait for the echo. 'echo', 'reset', 'eof'
    or 'timeout'."""
    try:
        with socket.create_connection(('127.0.0.1', port), timeout=timeout) as s:
            s.settimeout(timeout)
            s.sendall(b'0123456789abcdef')
            got = b''
            while len(got) < 16:
                chunk = s.recv(64)
                if not chunk:
                    return 'eof'
                got += chunk
            return 'echo'
    except ConnectionResetError:
        return 'reset'
    except (TimeoutError, socket.timeout):
        return 'timeout'
    except OSError as exc:
        return type(exc).__name__


class VisLab(ServiceGate):
    WOLVES = ('a', 'b', 'c')

    def guard(self):
        big = {p.name: p.stat().st_size for p in self.base.glob('*.log')
               if p.stat().st_size > LOG_CAP}
        if big:
            self.stop_wolves()
            raise LogCap(f'log cap exceeded, daemons stopped: {big}')

    def a_log(self):
        return b''.join(p.read_bytes() for p in sorted(self.base.glob('*-a-*.log')))

    def boot(self):
        self.prepare_binaries()
        self.reserve_wolf_ports()
        for key in ('fang', 'dead', 'dead2'):
            self.reserve(key)
        targets = self.base / 'targets'
        targets.mkdir()
        process, _ = self.spawn([sys.executable, ROOT / 'tests/integration/targets.py', targets],
                                'targets')
        ready = self.wait_for(lambda: json.loads((targets / 'ready.json').read_text()),
                              'targets', process)
        self.echo = f'127.0.0.1:{ready["echo"]}'
        for wolf in self.WOLVES:
            (self.base / wolf).mkdir(mode=0o700)
            write_json(self.base / wolf / 'target_policy.json',
                       {'mode': 'deny-by-default', 'peers': {}})
        self.start_wolves()
        self.keys = {}
        for wolf in self.WOLVES:
            self.ctl(wolf, 'pelt', 'init')
            self.keys[wolf] = self.ctl(wolf, 'pelt', 'show')['result']['public_key_b64']
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'), f'{wolf} TCP')
        # A dead port: reserved (bound, never listening) for TCP and UDP.
        self.dead = f'127.0.0.1:{self.ports["dead"]}'

    def pack(self, wolf, peer, address):
        return self.ctl_any(wolf, 'pack', 'add', f'wolf-{peer}', self.keys[peer], address)

    def ctl_any(self, wolf, *args):
        import subprocess
        argv = [str(self.control_binary), '--json', '--socket', str(self.base / f'{wolf}.sock'),
                *[str(a) for a in args]]
        result = subprocess.run(argv, cwd=self.base, env=self.env, capture_output=True,
                                text=True, timeout=60)
        lines = result.stdout.strip().splitlines()
        try:
            body = json.loads(lines[-1]) if lines else None
        except ValueError:
            body = lines[-1]
        return {'exit': result.returncode, 'body': body, 'stderr': result.stderr.strip()[-300:]}

    def fang_list(self):
        return self.control('a', 'fang.list')

    def connect(self, transport):
        self.release('fang')
        opened = self.ctl_any('a', 'service', 'connect', 'echo', '--local-port',
                              self.ports['fang'], '--transport', transport)
        if opened['exit'] == 0:
            self.wait_for(lambda: self.listener('fang'), 'fang listener')
        return opened

    def measure(self, count):
        """Run `count` clients one at a time; return outcomes and A's log
        during them."""
        before = len(self.a_log())
        fangs_before = self.fang_list()
        outcomes = {}
        start = time.monotonic()
        for i in range(count):
            if i % 10 == 0:
                self.guard()
            result = attempt(self.ports['fang'])
            outcomes[result] = outcomes.get(result, 0) + 1
        elapsed = round(time.monotonic() - start, 1)
        time.sleep(1)
        added = self.a_log()[before:]
        lines = added.decode(errors='replace').splitlines()
        return {'attempts': count, 'outcomes': outcomes, 'seconds': elapsed,
                'a_log_lines': len(lines), 'a_log_bytes': len(added),
                'a_log_sample': [line[:240] for line in lines[:4] + lines[-2:]] if lines else [],
                'fang_list_before': fangs_before, 'fang_list_after': self.fang_list(),
                'daemons_alive': all(w.poll() is None for w in self.wolves)}


def scenario_failure(lab, cause, kind, attempts):
    lab.boot()
    b_quic = f'quic://{lab.address("b_quic")}'
    c_quic = f'quic://{lab.address("c_quic")}'
    lab.pack('a', 'b', b_quic)
    lab.pack('a', 'c', f'tcp://{lab.address("c_tcp")}' if kind == 'routed' else c_quic)
    for wolf in ('b', 'c'):
        lab.pack(wolf, 'a', f'quic://{lab.address("a_quic")}')
    transport = {'quic': 'quic', 'tcp': 'tcp', 'tcp-plain': 'tcp-plain',
                 'routed': 'tcp', 'failover': 'quic'}[kind]
    target = lab.dead if (kind == 'tcp-plain' and cause == 'dial') else lab.echo
    peer = 'wolf-c' if kind == 'routed' else 'wolf-b'
    lab.ctl('a', 'service', 'add', 'echo', peer, target, 'tcp')
    if kind == 'routed':
        lab.ctl('a', 'route', 'set', 'wolf-c', '--via', 'wolf-b')
    if kind == 'failover':
        lab.ctl('a', 'service', 'provider', 'add', 'echo', 'wolf-c', target)
    if cause == 'grant':
        # Everything reachable; only the final receiver withholds its grant.
        if kind == 'routed':
            lab.ctl('b', 'target', 'allow', 'wolf-a', lab.address('c_tcp'))
    elif cause == 'dial' and kind != 'tcp-plain':
        # The first hop is dead: B's address (relay, or provider 1) and, for
        # failover, C's as well.
        lab.ctl('a', 'pack', 'set-address', 'wolf-b', f'quic://{lab.dead}')
        if kind == 'failover':
            lab.ctl('a', 'pack', 'set-address', 'wolf-c',
                    f'quic://127.0.0.1:{lab.ports["dead2"]}')
    opened = lab.connect(transport)
    require(opened['exit'] == 0, f'Fang did not open: {opened}')
    result = {'scenario': f'{cause}:{kind}', 'transport': transport,
              'open': opened['body'].get('result')}
    result.update(lab.measure(attempts))
    return result


def scenario_flood(lab, count):
    lab.boot()
    lab.ctl('a', 'service', 'add', 'echo', 'wolf-b', lab.dead, 'tcp')
    lab.pack('a', 'b', f'quic://{lab.address("b_quic")}')
    opened = lab.connect('tcp-plain')
    require(opened['exit'] == 0, f'Fang did not open: {opened}')
    result = {'scenario': 'flood:tcp-plain'}
    result.update(lab.measure(count))
    result.pop('fang_list_before')
    return result


def scenario_hostname(lab, kind, attempts):
    lab.boot()
    port = lab.ports['b_quic']
    added = lab.pack('a', 'b', f'{"quic" if kind == "quic" else "tcp"}://localhost:{port}')
    lab.pack('b', 'a', f'quic://{lab.address("a_quic")}')
    result = {'scenario': f'hostname:{kind}', 'pack_add': {k: added[k] for k in ('exit', 'stderr')}}
    if added['exit'] != 0:
        return result
    lab.ctl('a', 'service', 'add', 'echo', 'wolf-b', lab.echo, 'tcp')
    lab.ctl('b', 'target', 'allow', 'wolf-a', lab.echo)
    opened = lab.connect(kind)
    result['open'] = opened['body'].get('result') if opened['exit'] == 0 else opened
    if opened['exit'] == 0:
        result.update(lab.measure(attempts))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('scenarios', nargs='+', help='e.g. grant:quic dial:tcp-plain')
    parser.add_argument('--attempts', type=int, default=10)
    parser.add_argument('--flood', type=int, default=500)
    parser.add_argument('--out', required=True)
    parser.add_argument('--keep-temp', action='store_true')
    args = parser.parse_args()

    results = []
    for scenario in args.scenarios:
        cause, _, kind = scenario.partition(':')
        os.umask(0o077)
        lab = VisLab(args.keep_temp)
        try:
            if cause in ('grant', 'dial'):
                result = scenario_failure(lab, cause, kind, args.attempts)
            elif cause == 'flood':
                result = scenario_flood(lab, args.flood)
            elif cause == 'hostname':
                result = scenario_hostname(lab, kind, args.attempts)
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
    pathlib.Path(args.out).write_text(json.dumps(results, indent=1) + '\n')


if __name__ == '__main__':
    main()
