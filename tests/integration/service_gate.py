#!/usr/bin/env python3
"""Disposable checkout-only named-service gate (Stage23). See README.md.

Wolf A (consumer) defines services that resolve to targets behind wolf B
(provider) and connects to them with `werewolfctl service connect`. The gate
checks that the registry resolves intent but never grants authority: no
application byte reaches a target until B's own target policy allows it.
"""
import hashlib
import http.client
import json
import pathlib
import socket
import subprocess
import sys
import threading

sys.dont_write_bytecode = True
from run import Lab, main, require, write_json
from targets import MARKER, SSH_BANNER

REJECTED = 70  # werewolfctl ExitClass::Rejected


class ServiceGate(Lab):
    def ctl(self, wolf, *args, expect=0):
        argv = [str(self.control_binary), '--json', '--socket', str(self.base / f'{wolf}.sock'),
                *[str(a) for a in args]]
        result = subprocess.run(argv, cwd=self.base, env=self.env, capture_output=True,
                                text=True, timeout=30)
        with (self.base / 'cli.jsonl').open('a') as log:
            log.write(json.dumps({'wolf': wolf, 'argv': argv[1:], 'exit': result.returncode,
                                  'stdout': result.stdout, 'stderr': result.stderr}) + '\n')
        require(result.returncode == expect,
                f'{wolf} {" ".join(argv[4:])}: exit {result.returncode}, expected {expect}: '
                f'{result.stderr.strip()}')
        lines = result.stdout.strip().splitlines()
        return json.loads(lines[-1]) if lines else {}

    def active_fangs(self, wolf):
        return self.ctl(wolf, 'fang', 'active')['result']

    def http_get(self, key):
        connection = http.client.HTTPConnection('127.0.0.1', self.ports[key], timeout=12)
        try:
            connection.request('GET', '/baseline.txt')
            response = connection.getresponse()
            return response.status, response.read()
        finally:
            connection.close()

    def http_blocked(self, key):
        try:
            status, body = self.http_get(key)
        except (OSError, http.client.HTTPException) as exc:
            return {'error': type(exc).__name__}
        require(body != MARKER, f'{key}: target bytes delivered without target authorization')
        return {'status': status, 'bytes': len(body)}

    def duplex(self, key, total=8 << 20):
        block = hashlib.sha256(key.encode()).digest() * 2048
        sent, received = hashlib.sha256(), hashlib.sha256()
        count = 0
        with socket.create_connection(('127.0.0.1', self.ports[key]), timeout=20) as conn:
            def send():
                for _ in range(total // len(block)):
                    conn.sendall(block)
                    sent.update(block)
                conn.shutdown(socket.SHUT_WR)
            sender = threading.Thread(target=send)
            sender.start()
            while data := conn.recv(65536):
                received.update(data)
                count += len(data)
            sender.join()
        require(count == total, f'{key}: echoed {count} of {total} bytes')
        require(sent.digest() == received.digest(), f'{key}: SHA-256 mismatch')
        return {'bytes': count, 'sha256': received.hexdigest()}

    def ssh_banner(self, key):
        with socket.create_connection(('127.0.0.1', self.ports[key]), timeout=12) as conn:
            banner = conn.recv(256)
            conn.sendall(b'SSH-2.0-WerewolfGateClient\r\n')
        require(banner == SSH_BANNER, f'{key}: unexpected banner {banner!r}')
        return {'banner': banner.decode().strip()}

    def run(self):
        self.prepare_binaries()
        self.reserve_wolf_ports()
        for key in ('http_quic', 'http_tcp', 'echo_quic', 'echo_tcp', 'ssh', 'ghost',
                    'silver'):
            self.reserve(key)
        target_dir = self.base / 'targets'
        target_dir.mkdir()
        (target_dir / 'baseline.txt').write_bytes(MARKER)
        target, _ = self.spawn([sys.executable, pathlib.Path(__file__).with_name('targets.py'),
                                target_dir], 'targets')
        ready = self.wait_for(lambda: json.loads((target_dir / 'ready.json').read_text()),
                              'targets', target)
        targets = {kind: f'127.0.0.1:{ready[kind]}' for kind in ('http', 'echo', 'ssh')}

        # Both receivers start deny-by-default; only B's explicit grants open targets.
        for wolf in ('a', 'b'):
            (self.base / wolf).mkdir(mode=0o700)
            write_json(self.base / wolf / 'target_policy.json',
                       {'mode': 'deny-by-default', 'peers': {}})
        self.start_wolves()
        keys = {}
        for wolf in ('a', 'b'):
            self.ctl(wolf, 'pelt', 'init')
            keys[wolf] = self.ctl(wolf, 'pelt', 'show')['result']['public_key_b64']
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'), f'{wolf} TCP')
        self.ctl('a', 'pack', 'add', 'wolf-b', keys['b'], f'quic://{self.address("b_quic")}')
        self.ctl('b', 'pack', 'add', 'wolf-a', keys['a'], f'quic://{self.address("a_quic")}')

        definitions = {'web': ('wolf-b', targets['http'], 'http'),
                       'echo': ('wolf-b', targets['echo'], 'tcp'),
                       'shell': ('wolf-b', targets['ssh'], 'ssh'),
                       'ghost': ('wolf-unknown', targets['http'], 'http')}
        for name, (peer, target_address, kind) in definitions.items():
            self.ctl('a', 'service', 'add', name, peer, target_address, kind)
        self.ctl('a', 'service', 'add', 'web', 'wolf-b', targets['http'], 'http', expect=REJECTED)
        listed = {s['name']: s for s in self.ctl('a', 'service', 'list')['result']}
        require(sorted(listed) == sorted(definitions), f'service list mismatch: {sorted(listed)}')
        shown = self.ctl('a', 'service', 'show', 'web')['result']
        require(shown == {'name': 'web', 'peer': 'wolf-b', 'target': targets['http'], 'kind': 'http'},
                f'service show mismatch: {shown}')
        self.record('service registry CLI round trip',
                    {'services': sorted(listed), 'duplicate_add_exit': REJECTED})

        self.ctl('a', 'service', 'connect', 'ghost', '--local-port', self.ports['ghost'],
                 expect=REJECTED)
        self.ctl('a', 'service', 'connect', 'missing', '--local-port', self.ports['ghost'],
                 expect=REJECTED)
        require(self.active_fangs('a') == [], 'rejected service connect left an active Fang')
        self.record('unknown peer and missing service fail closed',
                    {'ghost_exit': REJECTED, 'missing_exit': REJECTED, 'active_fangs': 0})

        connects = {'http_quic': ('web', 'quic'), 'http_tcp': ('web', 'tcp'),
                    'echo_quic': ('echo', 'quic'), 'echo_tcp': ('echo', 'tcp'),
                    'ssh': ('shell', 'quic')}
        for key, (name, transport) in connects.items():
            self.release(key)
            fang = self.ctl('a', 'service', 'connect', name, '--local-port', self.ports[key],
                            '--transport', transport)['result']
            require(fang['remote'] == listed[name]['target'] and fang['peer'] == 'wolf-b'
                    and fang['transport'] == transport, f'{key}: unexpected Fang {fang}')
            self.wait_for(lambda key=key: self.listener(key), f'{key} listener')

        blocked = {key: self.http_blocked(key) for key in ('http_quic', 'http_tcp')}
        self.record('registry grants no target authority', blocked)

        for kind in ('http', 'echo', 'ssh'):
            self.ctl('b', 'target', 'allow', 'wolf-a', targets[kind])
        for key in ('http_quic', 'http_tcp'):
            status, body = self.http_get(key)
            require(status == 200 and body == MARKER, f'{key}: HTTP {status}, {len(body)} bytes')
            self.record(f'named HTTP service over {key.split("_")[1]}',
                        {'status': status, 'exact_payload': True})
        for key in ('echo_quic', 'echo_tcp'):
            self.record(f'named TCP service full duplex over {key.split("_")[1]}',
                        self.duplex(key))
        self.record('named SSH service banner over quic', self.ssh_banner('ssh'))

        before = self.ctl('a', 'service', 'list')['result']
        self.stop_wolves()
        self.start_wolves()
        after = self.ctl('a', 'service', 'list')['result']
        require(after == before, 'service registry changed across daemon restart')
        self.record('service registry survives daemon restart', {'services': len(after)})

        self.ctl('a', 'silver', 'on')
        require(self.active_fangs('a') == [], 'Silver left an active Fang')
        self.release('silver')
        self.ctl('a', 'service', 'connect', 'web', '--local-port', self.ports['silver'],
                 expect=REJECTED)
        require(self.active_fangs('a') == [], 'service connect opened a Fang under Silver')
        self.record('Silver fences service connect', {'exit': REJECTED, 'active_fangs': 0})
        require(all(p.poll() is None for p in [target, *self.wolves]), 'gate process exited')


if __name__ == '__main__':
    sys.exit(main(ServiceGate))
