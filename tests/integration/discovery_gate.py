#!/usr/bin/env python3
"""Disposable checkout-only discovery gate (Stage24). See README.md.

Three wolves: A trusts B, B trusts A and C, C trusts B. A finds C's signed
record through B over QUIC and encrypted TCP, yet discovery grants nothing:
A cannot reach C until both operators add each other to their Packs and C
allows the target.
"""
import json
import pathlib
import sys

sys.dont_write_bytecode = True
from run import main, require, write_json
from service_gate import REJECTED, ServiceGate


class DiscoveryGate(ServiceGate):
    WOLVES = ('a', 'b', 'c')

    def find(self, wolf, node, transport='quic'):
        return self.ctl(wolf, 'discover', 'find', node, '--transport', transport)['result']

    def run(self):
        self.prepare_binaries()
        self.reserve_wolf_ports()
        for key in ('echo', 'blocked'):
            self.reserve(key)
        target_dir = self.base / 'targets'
        target_dir.mkdir()
        target, _ = self.spawn([sys.executable, pathlib.Path(__file__).with_name('targets.py'),
                                target_dir], 'targets')
        ready = self.wait_for(lambda: json.loads((target_dir / 'ready.json').read_text()),
                              'targets', target)
        echo_target = f'127.0.0.1:{ready["echo"]}'

        for wolf in self.WOLVES:
            (self.base / wolf).mkdir(mode=0o700)
            write_json(self.base / wolf / 'target_policy.json',
                       {'mode': 'deny-by-default', 'peers': {}})
        self.start_wolves()
        keys, addresses = {}, {}
        for wolf in self.WOLVES:
            self.ctl(wolf, 'pelt', 'init')
            keys[wolf] = self.ctl(wolf, 'pelt', 'show')['result']['public_key_b64']
            addresses[wolf] = self.address(wolf + '_quic')
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'), f'{wolf} TCP')
        for wolf, peers in {'a': ('b',), 'b': ('a', 'c'), 'c': ('b',)}.items():
            for peer in peers:
                self.ctl(wolf, 'pack', 'add', f'wolf-{peer}', keys[peer],
                         f'quic://{addresses[peer]}')

        # C advertises both transports; the record is signed by C's Pelt.
        advertised = [f'quic://{addresses["c"]}', f'tcp://{addresses["c"]}']
        self.ctl('c', 'discover', 'advertise', *advertised)
        self.ctl('c', 'discover', 'advertise', 'tcp://wolf-c.example:1', expect=REJECTED)
        shown = self.ctl('c', 'discover', 'show')['result']
        require(shown['record']['addresses'] == advertised, 'C advertises the wrong addresses')
        node_c = shown['node_id']
        self.record('advertise signs an own record; hostnames rejected',
                    {'node_id': node_c, 'seq': shown['record']['seq']})

        refreshed = self.ctl('b', 'discover', 'refresh')['result']
        statuses = {s['peer']: s for s in refreshed['queried']}
        require(statuses['wolf-c'] == {'peer': 'wolf-c', 'status': 'ok', 'records': 1},
                f'B did not learn C: {statuses}')
        require(statuses['wolf-a']['status'] == 'ok' and statuses['wolf-a']['records'] == 0,
                f'A advertises nothing yet answered {statuses["wolf-a"]}')
        self.record('B pings its Pack and caches C', statuses)

        found = {}
        for transport, node in (('quic', keys['c']), ('tcp', node_c)):
            result = self.find('a', node, transport)
            record = result['found']
            require(record is not None, f'A did not find C over {transport}: {result}')
            require(record['public_key_b64'] == keys['c'] and record['node_id'] == node_c,
                    f'{transport}: wrong record')
            require(record['addresses'] == advertised, f'{transport}: wrong addresses')
            require(record['pack_name'] is None, f'{transport}: C reported as Pack member')
            found[transport] = record
            self.record(f'A finds C through B over {transport}',
                        {'queried': result['queried'], 'seq': record['seq']})
        require(found['quic'] == found['tcp'], 'QUIC and TCP returned different records')

        # Discovery grants nothing: C is cached, not trusted, and unreachable.
        pack = self.ctl('a', 'pack', 'list')['result']
        require([p['name'] for p in pack] == ['wolf-b'], f'discovery changed the Pack: {pack}')
        contacts = self.ctl('a', 'discover', 'contacts')['result']
        require([c['public_key_b64'] for c in contacts] == [keys['c']], 'A did not cache C')
        self.ctl('a', 'service', 'add', 'echo-c', 'wolf-c', echo_target, 'tcp')
        self.release('blocked')
        self.ctl('a', 'service', 'connect', 'echo-c', '--local-port', self.ports['blocked'],
                 expect=REJECTED)
        require(self.active_fangs('a') == [], 'a Fang opened to an undiscovered-only peer')
        self.record('discovered node is not trusted', {'pack': ['wolf-b'], 'exit': REJECTED})

        # Restart: the advertised addresses and the cache persist.
        self.stop_wolves()
        self.start_wolves()
        after = self.ctl('a', 'discover', 'contacts')['result']
        require([c['public_key_b64'] for c in after] == [keys['c']], 'cache lost on restart')
        require(self.ctl('c', 'discover', 'show')['result']['record']['addresses'] == advertised,
                'advertised addresses lost on restart')
        self.record('advertise and cache survive daemon restart', {'contacts': len(after)})

        # Trust stays a deliberate operator step on both sides.
        self.ctl('a', 'pack', 'add', 'wolf-c', found['quic']['public_key_b64'],
                 found['quic']['addresses'][0])
        self.ctl('c', 'pack', 'add', 'wolf-a', keys['a'], f'quic://{addresses["a"]}')
        self.ctl('c', 'target', 'allow', 'wolf-a', echo_target)
        self.release('echo')
        self.ctl('a', 'service', 'connect', 'echo-c', '--local-port', self.ports['echo'])
        self.record('discovered address works after pack add and target allow',
                    self.duplex('echo', total=1 << 20))

        # B stops vouching for C once C leaves B's Pack, although B still
        # caches C's record.
        self.ctl('b', 'pack', 'remove', 'wolf-c')
        result = self.find('a', node_c)
        statuses = {s['peer']: s for s in result['queried']}
        require(statuses['wolf-b'] == {'peer': 'wolf-b', 'status': 'ok', 'records': 0},
                f'B relayed a node outside its Pack: {statuses}')
        self.record('B relays only its own Pack members', statuses)

        self.ctl('a', 'silver', 'on')
        self.ctl('a', 'discover', 'find', node_c, expect=REJECTED)
        self.ctl('a', 'discover', 'refresh', expect=REJECTED)
        self.record('Silver fences discovery', {'exit': REJECTED})
        require(all(p.poll() is None for p in [target, *self.wolves]), 'gate process exited')


if __name__ == '__main__':
    sys.exit(main(DiscoveryGate))
