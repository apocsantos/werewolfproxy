#!/usr/bin/env python3
"""Disposable checkout-only service discovery gate (Stage25). See README.md.

Wolf A (provider) publishes services; wolves B and C (requesters) are both in
A's Pack, but A's Target Policy grants the echo target to B only. B sees the
service, imports it with `service add --from` and connects through it; C sees
nothing. Publishing grants nothing and discovery stores nothing: revoking the
grant hides the service again.
"""
import json
import pathlib
import sys

sys.dont_write_bytecode = True
from run import main, require, write_json
from service_gate import REJECTED, ServiceGate


class ServiceDiscoveryGate(ServiceGate):
    WOLVES = ('a', 'b', 'c')

    def offered(self, wolf, transport='quic'):
        result = self.ctl(wolf, 'discover', 'services', '--transport', transport)['result']
        statuses = {s['peer']: s for s in result['queried']}
        status = statuses['wolf-a']
        require(status['status'] == 'ok', f'{wolf} could not ask A over {transport}: {status}')
        return [(s['name'], s['kind'], s['target']) for s in status['services']]

    def run(self):
        self.prepare_binaries()
        self.reserve_wolf_ports()
        self.reserve('echo')
        target_dir = self.base / 'targets'
        target_dir.mkdir()
        target, _ = self.spawn([sys.executable, pathlib.Path(__file__).with_name('targets.py'),
                                target_dir], 'targets')
        ready = self.wait_for(lambda: json.loads((target_dir / 'ready.json').read_text()),
                              'targets', target)
        echo_target = f'127.0.0.1:{ready["echo"]}'
        http_target = f'127.0.0.1:{ready["http"]}'

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
        for wolf, peers in {'a': ('b', 'c'), 'b': ('a',), 'c': ('a',)}.items():
            for peer in peers:
                self.ctl(wolf, 'pack', 'add', f'wolf-{peer}', keys[peer],
                         f'quic://{self.address(peer + "_quic")}')

        # A publishes; nothing is granted yet, so nobody can see anything.
        self.ctl('a', 'service', 'publish', 'echo', echo_target, 'tcp')
        self.ctl('a', 'service', 'publish', 'web', http_target, 'http')
        self.ctl('a', 'service', 'publish', 'db', 'db.internal:5432', 'tcp', expect=REJECTED)
        self.ctl('a', 'service', 'publish', 'echo', echo_target, 'tcp', expect=REJECTED)
        published = self.ctl('a', 'service', 'published')['result']
        require([(p['name'], p['visible_to']) for p in published] == [('echo', []), ('web', [])],
                f'unexpected publication list: {published}')
        require(self.offered('b') == [] and self.offered('c') == [],
                'a service was shown without a target grant')
        self.record('publishing grants nothing', {'published': ['echo', 'web'], 'visible': 0})

        # A grants the echo target to B only.
        self.ctl('a', 'target', 'allow', 'wolf-b', echo_target)
        published = self.ctl('a', 'service', 'published')['result']
        require([(p['name'], p['visible_to']) for p in published]
                == [('echo', ['wolf-b']), ('web', [])], f'visibility mismatch: {published}')
        for transport in ('quic', 'tcp'):
            require(self.offered('b', transport) == [('echo', 'tcp', echo_target)],
                    f'B over {transport}: {self.offered("b", transport)}')
            require(self.offered('c', transport) == [], f'C saw a target granted only to B')
        self.record('only the granted requester sees the service',
                    {'b': ['echo'], 'c': [], 'transports': ['quic', 'tcp']})

        # Discovery stores nothing; importing is a deliberate step.
        require(self.ctl('b', 'service', 'list')['result'] == [], 'discovery wrote the registry')
        self.ctl('b', 'service', 'add', 'echo', '--from', 'wolf-a')
        self.ctl('b', 'service', 'add', 'web', '--from', 'wolf-a', expect=REJECTED)
        self.ctl('c', 'service', 'add', 'echo', '--from', 'wolf-a', expect=REJECTED)
        imported = self.ctl('b', 'service', 'list')['result']
        require(imported == [{'name': 'echo', 'peer': 'wolf-a', 'target': echo_target,
                              'kind': 'tcp'}], f'unexpected import: {imported}')
        require(self.ctl('c', 'service', 'list')['result'] == [], 'C imported a hidden service')
        self.record('service add --from imports only what the provider shows',
                    {'b': imported, 'c': [], 'not_granted_exit': REJECTED})

        self.release('echo')
        self.ctl('b', 'service', 'connect', 'echo', '--local-port', self.ports['echo'])
        self.wait_for(lambda: self.listener('echo'), 'echo listener')
        self.record('imported service connects through the normal Fang path',
                    self.duplex('echo', total=1 << 20))

        # Revoking the grant hides the service at once; B's registry entry
        # stays (it is only intent), and a new import is refused.
        self.ctl('a', 'target', 'remove', 'wolf-b', echo_target)
        require(self.offered('b') == [], 'a revoked target stayed visible')
        self.ctl('b', 'service', 'add', 'echo2', '--from', 'wolf-a', expect=REJECTED)
        self.record('revoking the grant hides the service', {'b': []})

        before = self.ctl('a', 'service', 'published')['result']
        self.stop_wolves()
        self.start_wolves()
        after = self.ctl('a', 'service', 'published')['result']
        require(after == before, 'publications changed across daemon restart')
        self.ctl('a', 'service', 'unpublish', 'web')
        self.ctl('a', 'service', 'unpublish', 'web', expect=REJECTED)
        require([p['name'] for p in self.ctl('a', 'service', 'published')['result']] == ['echo'],
                'unpublish did not remove the service')
        self.record('publications survive restart; unpublish removes',
                    {'after_restart': [p['name'] for p in after]})

        self.ctl('b', 'silver', 'on')
        self.ctl('b', 'discover', 'services', expect=REJECTED)
        self.ctl('b', 'service', 'add', 'echo3', '--from', 'wolf-a', expect=REJECTED)
        self.record('Silver fences service discovery', {'exit': REJECTED})
        require(all(p.poll() is None for p in [target, *self.wolves]), 'gate process exited')


if __name__ == '__main__':
    sys.exit(main(ServiceDiscoveryGate))
