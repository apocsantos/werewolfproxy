#!/usr/bin/env python3
"""Disposable checkout-only governance gate (Stage28). See README.md.

Ten governance keys are generated with `werewolfctl governance keygen`; every
statement is drafted, signed by seven holders and combined with the offline
commands, then applied to running daemons. Wolf A trusts B and C; B serves an
echo target to A. The gate checks genesis pinning, the manifest requirement,
a network revocation that closes A's Fang to B and survives restart, the
refusal to re-add a revoked key, replay, rotation, gaps, short signatures
and fork fencing.
"""
import json
import pathlib
import sys

sys.dont_write_bytecode = True
from run import main, require, write_json
from service_gate import REJECTED, ServiceGate

USAGE = 64


class GovernanceGate(ServiceGate):
    WOLVES = ('a', 'b', 'c')

    def gov(self, *args, expect=0):
        """An offline governance command (no daemon socket involved)."""
        return self.ctl('a', 'governance', *args, expect=expect)

    def statement(self, name, draft_args, signers):
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

        # Ten holders, a genesis signed by seven of them.
        self.gdir = self.base / 'governance'
        self.gdir.mkdir(mode=0o700)
        publics = [self.gov('keygen', self.gdir / f'k{i}.key')['result']['public_key_b64']
                   for i in range(10)]
        genesis = self.statement(
            'genesis', ['genesis', *[x for p in publics for x in ('--key', p)]], [0, 1, 2, 4, 5, 7, 9])
        genesis_hash = self.gov('inspect', genesis)['result']['statement']['hash']

        # C has no protected manifest yet: governance needs one.
        self.ctl('c', 'governance', 'apply', genesis, '--expect-hash', genesis_hash,
                 expect=REJECTED)
        for wolf in self.WOLVES:
            self.ctl(wolf, 'state', 'manifest-migrate')
        # Genesis must be pinned.
        self.ctl('a', 'governance', 'apply', genesis, expect=REJECTED)
        self.ctl('a', 'governance', 'apply', genesis, '--expect-hash', '00' * 32, expect=REJECTED)
        for wolf in self.WOLVES:
            applied = self.ctl(wolf, 'governance', 'apply', genesis, '--expect-hash', genesis_hash)
            require(applied['result']['status'] == 'applied', f'{wolf} genesis: {applied}')
        status = self.ctl('a', 'governance', 'status')['result']
        require(status['genesis'] and status['sequence'] == 0 and status['keys'] == publics
                and status['threshold'] == 7, f'status: {status}')
        manifest = sorted((self.base / 'a').glob('security_state_*_manifest.json'))
        require(any('"schema":2' in p.read_text() for p in manifest), 'manifest not schema 2')
        self.record('genesis is pinned, needs the manifest and commits as schema 2',
                    {'genesis_hash': genesis_hash, 'nodes': list(self.WOLVES)})

        # A trusts B and C; B serves an echo target to A.
        for peer in ('b', 'c'):
            self.ctl('a', 'pack', 'add', f'wolf-{peer}', keys[peer],
                     f'quic://{self.address(peer + "_quic")}')
            self.ctl(peer, 'pack', 'add', 'wolf-a', keys['a'], f'quic://{self.address("a_quic")}')
        self.ctl('b', 'target', 'allow', 'wolf-a', echo_target)
        self.ctl('a', 'service', 'add', 'echo', 'wolf-b', echo_target, 'tcp')
        self.release('echo')
        self.ctl('a', 'service', 'connect', 'echo', '--local-port', self.ports['echo'])
        self.wait_for(lambda: self.listener('echo'), 'echo listener')
        self.duplex('echo', total=1 << 20)

        # A statement signed by six holders is refused; seven revoke B.
        short = self.statement('short', ['revoke', '--after', genesis, '--member', keys['c']],
                               [0, 1, 2, 3, 4, 5])
        self.ctl('a', 'governance', 'apply', short, expect=REJECTED)
        revoke_b = self.statement('revoke-b', ['revoke', '--after', genesis, '--member', keys['b']],
                                  [3, 4, 5, 6, 7, 8, 9])
        applied = self.ctl('a', 'governance', 'apply', revoke_b)['result']
        require(applied['status'] == 'applied' and applied['revoked_members'] == ['wolf-b']
                and applied['closed_fangs'] == 1, f'revocation: {applied}')
        require(self.active_fangs('a') == [], 'Fang to the revoked member survived')
        pack = [p['name'] for p in self.ctl('a', 'pack', 'list')['result']]
        require(pack == ['wolf-c'], f'pack after revocation: {pack}')
        self.ctl('a', 'pack', 'add', 'wolf-b', keys['b'], f'quic://{self.address("b_quic")}',
                 expect=REJECTED)
        replay = self.ctl('a', 'governance', 'apply', revoke_b)['result']
        require(replay['status'] == 'already_applied', f'replay: {replay}')
        self.record('a network revocation removes the member and closes its Fangs',
                    {**applied, 'readd_exit': REJECTED, 'replay': replay['status']})

        # The head and the refusal survive a restart.
        self.stop_wolves()
        self.start_wolves()
        status = self.ctl('a', 'governance', 'status')['result']
        require(status['sequence'] == 1 and status['revoked'] == [keys['b']]
                and not status['forked'], f'status after restart: {status}')
        self.ctl('a', 'pack', 'add', 'wolf-b', keys['b'], f'quic://{self.address("b_quic")}',
                 expect=REJECTED)
        self.record('the governance head and revocation survive restart', status)

        # B is at sequence 0: a later statement is a gap. A rotation applies.
        later = self.statement('revoke-c', ['revoke', '--after', revoke_b, '--member', keys['c']],
                               [0, 1, 2, 3, 4, 5, 6])
        self.ctl('b', 'governance', 'apply', later, expect=REJECTED)
        new_key = self.gov('keygen', self.gdir / 'k9new.key')['result']['public_key_b64']
        rotate = self.statement('rotate', ['rotate', '--after', genesis, '--set', f'9={new_key}'],
                                [0, 1, 2, 3, 4, 5, 6])
        rotated = self.ctl('b', 'governance', 'apply', rotate)['result']
        require(rotated['status'] == 'applied', f'rotation: {rotated}')
        require(self.ctl('b', 'governance', 'status')['result']['keys'][9] == new_key,
                'rotation not in force')
        self.record('a gap is refused and a rotation applies', {'b_sequence': 1})

        # On A, the rotation conflicts with the applied revocation at
        # sequence 1: both validly signed, so governance is fenced.
        self.ctl('a', 'governance', 'apply', rotate, expect=REJECTED)
        status = self.ctl('a', 'governance', 'status')['result']
        require(status['forked'] and status['sequence'] == 1, f'fork not recorded: {status}')
        self.ctl('a', 'governance', 'apply', later, expect=REJECTED)
        self.stop_wolves()
        self.start_wolves()
        require(self.ctl('a', 'governance', 'status')['result']['forked'],
                'fork flag lost on restart')
        self.record('a validly signed conflict fences governance durably', {'forked': True})

        # Stage29 kinds are valid statements, but this daemon refuses them
        # until their node effects exist (Stage29 step 2).
        admit = self.statement('admit', ['admit', '--after', rotate, '--member', keys['a']],
                               [0, 1, 2, 3, 4, 5, 6])
        self.ctl('b', 'governance', 'apply', admit, expect=REJECTED)
        require(self.ctl('b', 'governance', 'status')['result']['sequence'] == 1,
                'an unsupported statement changed the head')
        self.record('admission is refused until its node effects exist', {'exit': REJECTED})
        require(all(p.poll() is None for p in [target, *self.wolves]), 'gate process exited')


if __name__ == '__main__':
    sys.exit(main(GovernanceGate))
