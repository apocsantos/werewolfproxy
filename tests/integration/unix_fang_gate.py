#!/usr/bin/env python3
"""Disposable Linux end-to-end gate for Unix local Fang streams.

Exercises QUIC, encrypted TCP, plain TCP, routed and provider-failover Fangs
through the same `unix:` local interface and verifies exact application bytes.
"""
import json
import pathlib
import socket
import sys

sys.dont_write_bytecode = True
from run import main, require, write_json
from service_gate import ServiceGate


class UnixFangGate(ServiceGate):
    WOLVES = ('a', 'b', 'c')

    def endpoint(self, name):
        return f'unix:{self.base / "sockets" / (name + ".sock")}'

    def connect(self, service, name, transport):
        path = self.base / 'sockets' / f'{name}.sock'
        require(not path.exists(), f'{name}: stale socket before connect')
        result = self.ctl('a', 'service', 'connect', service, '--local-endpoint',
                          self.endpoint(name), '--transport', transport)['result']
        self.wait_for(lambda: path.exists(), f'{name} Unix listener')
        require(result.get('fang_id'), f'{name}: control response missing Fang ID: {result}')
        return result['fang_id'], path

    @staticmethod
    def exchange(path, payload):
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.settimeout(20)
            client.connect(str(path))
            client.sendall(payload)
            client.shutdown(socket.SHUT_WR)
            chunks = []
            while chunk := client.recv(65536):
                chunks.append(chunk)
        echoed = b''.join(chunks)
        require(echoed == payload, f'{path.name}: application bytes differed')
        return {'bytes': len(echoed), 'exact': True}

    def close_endpoint(self, fang_id, path):
        self.ctl('a', 'fang', 'close', fang_id)
        self.wait_for(lambda: not path.exists(), f'{path.name} socket cleanup')

    def exercise(self, service, name, transport, payload):
        fang_id, path = self.connect(service, name, transport)
        result = self.exchange(path, payload)
        self.close_endpoint(fang_id, path)
        self.record(f'{name}: Unix stream over {transport}', result)

    def run(self):
        require(sys.platform.startswith('linux'), 'Unix Fang gate requires Linux SO_PEERCRED')
        self.prepare_binaries()
        self.reserve_wolf_ports()
        for wolf in self.WOLVES:
            (self.base / wolf).mkdir(mode=0o700)
            write_json(self.base / wolf / 'target_policy.json',
                       {'mode': 'deny-by-default', 'peers': {}})
        sockets = self.base / 'sockets'
        sockets.mkdir(mode=0o750)
        sockets.chmod(0o2750)

        targets_dir = self.base / 'targets'
        targets_dir.mkdir()
        target, _ = self.spawn([sys.executable, pathlib.Path(__file__).with_name('targets.py'),
                                targets_dir], 'targets')
        ready = self.wait_for(lambda: json.loads((targets_dir / 'ready.json').read_text()),
                              'targets', target)
        echo_target = f'127.0.0.1:{ready["echo"]}'

        self.start_wolves()
        keys = {}
        for wolf in self.WOLVES:
            self.ctl(wolf, 'pelt', 'init')
            keys[wolf] = self.ctl(wolf, 'pelt', 'show')['result']['public_key_b64']
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'), f'{wolf} TCP')

        self.ctl('a', 'pack', 'add', 'wolf-b', keys['b'], f'quic://{self.address("b_quic")}')
        self.ctl('a', 'pack', 'add', 'wolf-c', keys['c'], f'tcp://{self.address("c_tcp")}')
        for wolf in ('b', 'c'):
            self.ctl(wolf, 'pack', 'add', 'wolf-a', keys['a'],
                     f'quic://{self.address("a_quic")}')

        # A uses B both as a direct provider and as the relay to C. B may
        # relay A only to C's TCP listener; C separately grants A the target.
        self.ctl('b', 'target', 'allow', 'wolf-a', echo_target)
        self.ctl('b', 'target', 'allow', 'wolf-a', self.address('c_tcp'))
        self.ctl('c', 'target', 'allow', 'wolf-a', echo_target)

        self.ctl('a', 'service', 'add', 'b-echo', 'wolf-b', echo_target, 'tcp')
        self.ctl('a', 'service', 'add', 'c-echo', 'wolf-c', echo_target, 'tcp')
        # The first provider refuses the deliberately ungranted target; C is
        # the authorized fallback and receives the application stream.
        self.ctl('a', 'service', 'add', 'fallback', 'wolf-b', '127.0.0.1:1', 'tcp')
        self.ctl('a', 'service', 'provider', 'add', 'fallback', 'wolf-c', echo_target)
        self.ctl('a', 'route', 'set', 'wolf-c', '--via', 'wolf-b')

        self.exercise('b-echo', 'direct-quic', 'quic', b'quic-unix-stream\n')

        # The direct TCP and plain TCP dials use B's encrypted TCP address.
        changed = self.ctl('a', 'pack', 'set-address', 'wolf-b',
                           f'tcp://{self.address("b_tcp")}')['result']
        require(changed.get('closed_fangs') == 0, f'unexpected Fangs closed: {changed}')
        self.exercise('b-echo', 'direct-tcp', 'tcp', b'encrypted-tcp-unix-stream\n')
        self.exercise('b-echo', 'direct-plain', 'tcp-plain', b'plain-tcp-unix-stream\n')

        # Routed and fallback paths need QUIC addresses for B and C's TCP
        # listener as the routed destination.
        self.ctl('a', 'pack', 'set-address', 'wolf-b', f'quic://{self.address("b_quic")}')
        self.exercise('c-echo', 'routed-quic-hop', 'quic', b'routed-unix-stream\n')
        self.exercise('fallback', 'provider-failover', 'quic', b'failover-unix-stream\n')

        require(all(process.poll() is None for process in [target, *self.wolves]),
                'a daemon or target exited during Unix Fang traffic')
        self.record('all five Unix Fang transport paths remained live',
                    {'paths': ['QUIC', 'encrypted TCP', 'plain TCP', 'routed', 'provider failover']})


if __name__ == '__main__':
    main(UnixFangGate)
