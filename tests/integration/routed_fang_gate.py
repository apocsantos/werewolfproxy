#!/usr/bin/env python3
"""Disposable checkout-only routed Fang gate (Stage26). See README.md.

Wolf A reaches wolf C's echo target through relay B. A's route says
"wolf-c via wolf-b"; B's Target Policy grants A only C's encrypted-TCP
listener; C's Target Policy grants A the echo target. The inner session is
end to end: it needs both grants, either one alone fails, Silver on the relay
cuts a live session, and plain TCP is never carried through a relay.
"""
import json
import pathlib
import socket
import sys
import time

sys.dont_write_bytecode = True
from run import main, require, write_json
from service_gate import REJECTED, ServiceGate


class RoutedFangGate(ServiceGate):
    WOLVES = ('a', 'b', 'c')

    def authority(self, wolf):
        return self.ctl(wolf, 'status')['result']['inbound_authority']['active']

    def connect_service(self, name, key, *extra, expect=0):
        self.release(key)
        result = self.ctl('a', 'service', 'connect', name, '--local-port', self.ports[key],
                          *extra, expect=expect)
        if expect == 0:
            self.wait_for(lambda: self.listener(key), f'{key} listener')
        return result.get('result', {})

    def close_all(self):
        for fang in self.active_fangs('a'):
            self.ctl('a', 'fang', 'close', fang['id'])

    def blocked(self, key):
        """A session through the Fang must deliver nothing and end."""
        with socket.create_connection(('127.0.0.1', self.ports[key]), timeout=15) as conn:
            try:
                conn.sendall(b'routed-probe\n')
                data = conn.recv(4096)
            except (ConnectionResetError, BrokenPipeError) as exc:
                return {'end': type(exc).__name__}
        require(data == b'', f'{key}: bytes delivered without both grants: {data!r}')
        return {'end': 'eof'}

    def held(self, key):
        conn = socket.create_connection(('127.0.0.1', self.ports[key]), timeout=15)
        conn.sendall(b'held\n')
        require(conn.recv(64) == b'held\n', f'{key}: held session did not echo')
        return conn

    def run(self):
        self.prepare_binaries()
        self.reserve_wolf_ports()
        for key in ('quic_hop', 'tcp_hop', 'no_relay_grant', 'no_dest_grant', 'held', 'after'):
            self.reserve(key)
        target_dir = self.base / 'targets'
        target_dir.mkdir()
        target, _ = self.spawn([sys.executable, pathlib.Path(__file__).with_name('targets.py'),
                                target_dir], 'targets')
        ready = self.wait_for(lambda: json.loads((target_dir / 'ready.json').read_text()),
                              'targets', target)
        echo_target = f'127.0.0.1:{ready["echo"]}'
        c_listener = self.address('c_tcp')

        for wolf in self.WOLVES:
            (self.base / wolf).mkdir(mode=0o700)
            write_json(self.base / wolf / 'target_policy.json',
                       {'mode': 'deny-by-default', 'peers': {}})
        self.start_wolves()
        keys = {}
        for wolf in self.WOLVES:
            self.ctl(wolf, 'pelt', 'init')
            keys[wolf] = self.ctl(wolf, 'pelt', 'show')['result']['public_key_b64']
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'), f'{wolf} TCP')
        # A knows B (relay) and C (destination, by its TCP listener); B and C
        # know A. B and C do not need to know each other.
        self.ctl('a', 'pack', 'add', 'wolf-b', keys['b'], f'quic://{self.address("b_quic")}')
        self.ctl('a', 'pack', 'add', 'wolf-c', keys['c'], f'tcp://{c_listener}')
        for wolf in ('b', 'c'):
            self.ctl(wolf, 'pack', 'add', 'wolf-a', keys['a'], f'quic://{self.address("a_quic")}')
        self.ctl('a', 'service', 'add', 'echo', 'wolf-c', echo_target, 'tcp')

        # Route set; nothing granted yet on either hop.
        self.ctl('a', 'route', 'set', 'wolf-c', '--via', 'wolf-b')
        self.connect_service('echo', 'no_relay_grant')
        self.ctl('c', 'target', 'allow', 'wolf-a', echo_target)
        no_relay = self.blocked('no_relay_grant')
        self.ctl('c', 'target', 'remove', 'wolf-a', echo_target)
        self.ctl('b', 'target', 'allow', 'wolf-a', c_listener)
        self.connect_service('echo', 'no_dest_grant')
        no_dest = self.blocked('no_dest_grant')
        self.record('a routed session needs both the relay and the destination grant',
                    {'only_destination_grant': no_relay, 'only_relay_grant': no_dest})
        self.close_all()

        # Both grants: QUIC hop to the relay, encrypted TCP inside.
        self.ctl('c', 'target', 'allow', 'wolf-a', echo_target)
        opened = self.connect_service('echo', 'quic_hop')
        fangs = self.active_fangs('a')
        require(opened.get('via') == 'wolf-b' and opened.get('peer_addr') == c_listener,
                f'unexpected routed open: {opened}')
        require([f['transport'] for f in fangs] == ['tcp via wolf-b (quic)'],
                f'unexpected fang list: {fangs}')
        result = self.duplex('quic_hop', total=64 << 20)
        self.record('routed Fang over a QUIC hop, 64 MiB full duplex',
                    {**result, 'transport': fangs[0]['transport']})

        # While a session is open, both B and C hold one authority session.
        conn = self.held('quic_hop')
        holding = {'b': self.authority('b'), 'c': self.authority('c')}
        conn.close()
        require(holding == {'b': 1, 'c': 1}, f'expected one session on B and C: {holding}')
        self.record('the relay and the destination each authorize one session', holding)
        self.close_all()

        # TCP hop to the relay.
        self.ctl('a', 'route', 'set', 'wolf-c', '--via', 'wolf-b', '--hop', 'tcp')
        self.connect_service('echo', 'tcp_hop')
        require([f['transport'] for f in self.active_fangs('a')] == ['tcp via wolf-b (tcp)'],
                'TCP hop not shown')
        self.record('routed Fang over an encrypted TCP hop, 64 MiB full duplex',
                    self.duplex('tcp_hop', total=64 << 20))
        self.close_all()

        # Plain TCP is never carried through a relay.
        self.connect_service('echo', 'after', '--transport', 'tcp-plain', expect=REJECTED)
        require(self.active_fangs('a') == [], 'refused routed open left a Fang')
        self.record('plain TCP is refused for a routed peer', {'exit': REJECTED})

        # Silver on the relay cuts a live routed session; after Silver ends,
        # a new session works again.
        self.ctl('a', 'route', 'set', 'wolf-c', '--via', 'wolf-b')
        self.connect_service('echo', 'held')
        conn = self.held('held')
        self.ctl('b', 'silver', 'on')
        started = time.monotonic()
        try:
            data = conn.recv(64)
            end = 'eof' if data == b'' else f'data {data!r}'
        except (ConnectionResetError, socket.timeout) as exc:
            end = type(exc).__name__
        cut_after = round(time.monotonic() - started, 3)
        conn.close()
        require(end in ('ConnectionResetError', 'eof'), f'live session not cut: {end}')
        cut = {'end': end, 'secs': cut_after, 'relay_sessions': self.authority('b')}
        self.ctl('b', 'silver', 'off')
        self.record('Silver on the relay cuts a live routed session', cut)
        self.record('a new routed session works after Silver ends',
                    self.duplex('held', total=1 << 20))

        # A relay address change on A closes the routed Fang (LC-01 applies to
        # the relay as well as to the destination).
        conn = self.held('held')
        changed = self.ctl('a', 'pack', 'set-address', 'wolf-b',
                           f'tcp://{self.address("b_tcp")}')['result']
        try:
            data = conn.recv(64)
            end = 'eof' if data == b'' else f'data {data!r}'
        except (ConnectionResetError, socket.timeout) as exc:
            end = type(exc).__name__
        conn.close()
        left = self.active_fangs('a')
        require(changed['closed_fangs'] == 1 and left == [], f'routed Fang survived: {left}')
        require(end in ('ConnectionResetError', 'eof'), f'live session not cut: {end}')
        self.ctl('a', 'pack', 'set-address', 'wolf-b', f'quic://{self.address("b_quic")}')
        self.record('a relay address change closes the routed Fang',
                    {'closed_fangs': changed['closed_fangs'], 'end': end})

        # Removing the route turns the next Fang back into a direct one.
        self.close_all()
        self.ctl('a', 'route', 'remove', 'wolf-c')
        opened = self.connect_service('echo', 'after', '--transport', 'tcp')
        require(opened.get('via') is None, f'route removal ignored: {opened}')
        self.record('without the route the Fang is direct', {'peer_addr': opened.get('peer_addr')})
        require(all(p.poll() is None for p in [target, *self.wolves]), 'gate process exited')


if __name__ == '__main__':
    sys.exit(main(RoutedFangGate))
