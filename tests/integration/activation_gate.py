#!/usr/bin/env python3
"""Disposable checkout-only activation gate (Stage29). See README.md.

Three wolves share one governance network. A and B are admitted; X is not,
at first. Activation is a network-chain fact applied by every node; X then
advances its head but stays fenced until a later admission. The gate runs
the owner's two scenarios against real daemons: a node admitted after
activation, and permanent revocation.
"""
import json
import pathlib
import sys

sys.dont_write_bytecode = True
from run import main, require, write_json
from service_gate import REJECTED, ServiceGate


class ActivationGate(ServiceGate):
    WOLVES = ('a', 'b', 'x')

    def gov(self, *args, expect=0):
        return self.ctl('a', 'governance', *args, expect=expect)

    def statement(self, name, draft_args, signers=(0, 1, 2, 3, 4, 5, 6)):
        draft = self.gdir / f'{name}.draft'
        self.gov('draft', *draft_args, '--out', draft)
        parts = []
        for i in signers:
            part = self.gdir / f'{name}.{i}.part'
            self.gov('sign', draft, self.gdir / f'k{i}.key', '--index', i, '--out', part)
            parts.append(part)
        out = self.gdir / f'{name}.wwgov'
        self.gov('combine', draft, *parts, '--out', out)
        return out

    def apply_all(self, statement, *extra):
        return {w: self.ctl(w, 'governance', 'apply', statement, *extra)['result']
                for w in self.WOLVES}

    def status(self, wolf):
        return self.ctl(wolf, 'governance', 'status')['result']

    def locked(self, wolf):
        return self.ctl(wolf, 'status')['result']['inbound_authority']['locked']

    def connect_x(self):
        self.release('echo_x')
        self.ctl('a', 'service', 'connect', 'echo-x', '--local-port', self.ports['echo_x'])
        self.wait_for(lambda: self.listener('echo_x'), 'echo-x listener')
        return self.duplex('echo_x', total=1 << 20)

    def close_all(self, wolf):
        for fang in self.active_fangs(wolf):
            self.ctl(wolf, 'fang', 'close', fang['id'])

    def run(self):
        self.prepare_binaries()
        self.reserve_wolf_ports()
        self.reserve('echo_x')
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
        keys = {}
        for wolf in self.WOLVES:
            self.ctl(wolf, 'pelt', 'init')
            keys[wolf] = self.ctl(wolf, 'pelt', 'show')['result']['public_key_b64']
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'), f'{wolf} TCP')
            self.ctl(wolf, 'state', 'manifest-migrate')

        # One governance network; A trusts B and X; X serves echo to A.
        self.gdir = self.base / 'governance'
        self.gdir.mkdir(mode=0o700)
        publics = [self.gov('keygen', self.gdir / f'k{i}.key')['result']['public_key_b64']
                   for i in range(10)]
        genesis = self.statement('genesis', ['genesis', *[x for p in publics for x in ('--key', p)]])
        genesis_hash = self.gov('inspect', genesis)['result']['statement']['hash']
        self.apply_all(genesis, '--expect-hash', genesis_hash)
        for peer in ('b', 'x'):
            self.ctl('a', 'pack', 'add', f'wolf-{peer}', keys[peer],
                     f'quic://{self.address(peer + "_quic")}')
            self.ctl(peer, 'pack', 'add', 'wolf-a', keys['a'], f'quic://{self.address("a_quic")}')
        self.ctl('x', 'target', 'allow', 'wolf-a', echo_target)
        self.ctl('a', 'service', 'add', 'echo-x', 'wolf-x', echo_target, 'tcp')
        self.connect_x()
        require(all(self.status(w)['admitted'] for w in self.WOLVES), 'admitted before activation')
        self.record('before activation every node is eligible', {'echo_through_x': True})

        # Scenario 1: A and B admitted, then activation while X is not.
        admit_ab = self.statement('admit-ab', ['admit', '--after', genesis,
                                               '--member', keys['a'], '--member', keys['b']])
        self.apply_all(admit_ab)
        activate = self.statement('activate', ['activate', '--after', admit_ab])
        results = self.apply_all(activate)
        require(results['a']['revoked_members'] == ['wolf-x'] and results['a']['closed_fangs'] == 1,
                f'activation sweep on A: {results["a"]}')
        require(results['x']['admitted'] is False and results['a']['admitted'], f'{results}')
        heads = {w: (s['sequence'], s['hash']) for w, s in
                 ((w, self.status(w)) for w in self.WOLVES)}
        require(len(set(heads.values())) == 1 and heads['x'][0] == 2,
                f'X must advance its head like everyone: {heads}')
        require(self.locked('x') and not self.locked('a'), 'X not fenced, or A fenced')
        self.ctl('x', 'fang', 'open', 'wolf-a', '127.0.0.1:1', echo_target, expect=REJECTED)
        self.ctl('x', 'discover', 'refresh', expect=REJECTED)
        self.ctl('a', 'pack', 'add', 'wolf-x', keys['x'], f'quic://{self.address("x_quic")}',
                 expect=REJECTED)
        # Silver does not lift the fence.
        self.ctl('x', 'silver', 'on')
        self.ctl('x', 'silver', 'off')
        require(self.locked('x'), 'Silver reset lifted the admission fence')
        self.stop_wolves()
        self.start_wolves()
        require(self.locked('x') and self.status('x')['admitted'] is False,
                'X not fenced after restart')
        self.record('activation advances every head; X stays fenced, also after restart',
                    {'sequence': 2, 'swept_on_a': ['wolf-x'], 'x_locked': True})

        # A later admission of X makes it a member: nothing skipped.
        admit_x = self.statement('admit-x', ['admit', '--after', activate, '--member', keys['x']])
        results = self.apply_all(admit_x)
        require(results['x']['admitted'] and not self.locked('x'), f'X not admitted: {results}')
        require(self.status('x')['sequence'] == 3, 'unexpected sequence')
        self.ctl('x', 'discover', 'refresh')
        self.ctl('a', 'pack', 'add', 'wolf-x', keys['x'], f'quic://{self.address("x_quic")}')
        self.record('a later admission lifts the fence', {'echo_through_x': self.connect_x()})

        # Scenario 2: revocation is permanent.
        revoke_x = self.statement('revoke-x', ['revoke', '--after', admit_x, '--member', keys['x']])
        results = self.apply_all(revoke_x)
        require(results['a']['revoked_members'] == ['wolf-x'] and not results['x']['admitted']
                and self.locked('x'), f'revocation: {results}')
        readmit = self.statement('readmit-x', ['admit', '--after', revoke_x, '--member', keys['x']])
        for wolf in self.WOLVES:
            self.ctl(wolf, 'governance', 'apply', readmit, expect=REJECTED)
        require(all(self.status(w)['sequence'] == 4 for w in self.WOLVES), 'a head moved')
        self.ctl('a', 'pack', 'add', 'wolf-x', keys['x'], f'quic://{self.address("x_quic")}',
                 expect=REJECTED)
        self.record('a revoked key is never admitted again', {'readmit_exit': REJECTED})
        require(all(p.poll() is None for p in [target, *self.wolves]), 'gate process exited')


if __name__ == '__main__':
    sys.exit(main(ActivationGate))
