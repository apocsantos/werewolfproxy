#!/usr/bin/env python3
"""Disposable checkout-only governance sync gate (Stage29 step 3). See README.md.

Node C learns governance statements from its Pack members A and B with
`governance sync`. A and B receive statements as files; C only syncs. The
gate covers multi-round catch-up, identical offers applied once, a
revocation reaching C through a peer, the archive rules of G3 (an
uncommitted artefact beyond the head is replaced on apply; a tampered
archive entry is never served) and addition D (conflicting valid successors
fence C before either is applied).
"""
import json
import os
import pathlib
import sys

sys.dont_write_bytecode = True
from run import main, require, write_json
from service_gate import REJECTED, ServiceGate


class SyncGate(ServiceGate):
    WOLVES = ('a', 'b', 'c')

    def gov(self, *args, expect=0):
        return self.ctl('a', 'governance', *args, expect=expect)

    def statement(self, name, draft_args):
        draft = self.gdir / f'{name}.draft'
        self.gov('draft', *draft_args, '--out', draft)
        parts = []
        for i in range(7):
            part = self.gdir / f'{name}.{i}.part'
            self.gov('sign', draft, self.gdir / f'k{i}.key', '--index', i, '--out', part)
            parts.append(part)
        out = self.gdir / f'{name}.wwgov'
        self.gov('combine', draft, *parts, '--out', out)
        return out

    def member(self, i):
        # A fixed, valid Ed25519 public key that no wolf uses.
        return self.gov('keygen', self.gdir / f'member{i}.key')['result']['public_key_b64']

    def head(self, wolf):
        return self.ctl(wolf, 'governance', 'status')['result']

    def sync(self, wolf, expect=0):
        return self.ctl(wolf, 'governance', 'sync', expect=expect)

    def archive(self, wolf, sequence):
        return self.base / wolf / f'governance_{sequence:08}.wwgov'

    def run(self):
        self.prepare_binaries()
        self.reserve_wolf_ports()
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
        # C trusts A and B; A and B trust C, so they answer its RPCs.
        for peer in ('a', 'b'):
            self.ctl('c', 'pack', 'add', f'wolf-{peer}', keys[peer],
                     f'quic://{self.address(peer + "_quic")}')
            self.ctl(peer, 'pack', 'add', 'wolf-c', keys['c'], f'quic://{self.address("c_quic")}')

        self.gdir = self.base / 'governance'
        self.gdir.mkdir(mode=0o700)
        publics = [self.gov('keygen', self.gdir / f'k{i}.key')['result']['public_key_b64']
                   for i in range(10)]
        genesis = self.statement('genesis', ['genesis', *[x for p in publics for x in ('--key', p)]])
        genesis_hash = self.gov('inspect', genesis)['result']['statement']['hash']
        for wolf in self.WOLVES:
            self.ctl(wolf, 'governance', 'apply', genesis, '--expect-hash', genesis_hash)

        # A "ghost" member in C's Pack, to be revoked later through a peer.
        ghost = self.member(99)
        self.ctl('c', 'pack', 'add', 'ghost', ghost, 'quic://127.0.0.1:9')

        # Five admissions on A and B (files); C has only the genesis.
        chain, previous = [], genesis
        for i in range(5):
            previous = self.statement(f'admit{i}', ['admit', '--after', previous,
                                                    '--member', self.member(i)])
            chain.append(previous)
        for wolf in ('a', 'b'):
            for statement in chain:
                self.ctl(wolf, 'governance', 'apply', statement)
            require(all(self.archive(wolf, s).exists() for s in range(1, 6)),
                    f'{wolf}: an applied statement was not archived')
        # B's archive entry for sequence 3 is replaced by a valid statement
        # that is not the one B applied there: B must stop serving at 3.
        self.archive('b', 3).write_bytes(self.archive('b', 2).read_bytes())
        # G3: an uncommitted artefact beyond C's head (garbage at sequence 1).
        artefact = self.archive('c', 1)
        artefact.write_bytes(b'not a statement')
        os.chmod(artefact, 0o600)

        synced = self.sync('c')['result']
        applied = [s['sequence'] for s in synced['applied']]
        require(applied == [1, 2, 3, 4, 5] and 'stopped' not in synced, f'catch-up: {synced}')
        status = self.head('c')
        require(status['sequence'] == 5 and len(status['roster']) == 5, f'C head: {status}')
        require(artefact.read_bytes() != b'not a statement', 'artefact not replaced')
        require(artefact.read_bytes() == self.archive('a', 1).read_bytes(),
                'C archived a different sequence-1 statement')
        self.record('a five-statement catch-up over two rounds, applied once',
                    {'applied': applied, 'peers': synced['peers']})
        self.record('an uncommitted archive artefact beyond the head is replaced on apply',
                    {'sequence': 1})
        again = self.sync('c')['result']
        require(again['applied'] == [], f'resync applied again: {again}')

        # The tampered entry was never served: in round one (from sequence 0)
        # B offered sequences 1-2 and stopped at its mismatching entry 3, while
        # A offered 1-4; in round two (from 4) each offered sequence 5.
        offers = {p['peer']: p['offered'] for p in synced['peers'] if p['peer'] != 'ghost'}
        require(offers == {'wolf-a': [4, 1], 'wolf-b': [2, 1]}, f'offers: {offers}')
        ghost_status = {p['peer']: p for p in synced['peers']}['ghost']['status']
        require(ghost_status == 'unreachable', f'ghost: {ghost_status}')
        self.record('a tampered archive entry is never served', offers)

        # A revocation reaches C through a peer and removes the member.
        revoke = self.statement('revoke-ghost', ['revoke', '--after', previous,
                                                 '--member', ghost])
        for wolf in ('a', 'b'):
            self.ctl(wolf, 'governance', 'apply', revoke)
        synced = self.sync('c')['result']
        require([s['revoked_members'] for s in synced['applied']] == [['ghost']],
                f'revocation through a peer: {synced}')
        pack = sorted(p['name'] for p in self.ctl('c', 'pack', 'list')['result'])
        require(pack == ['wolf-a', 'wolf-b'], f'C pack: {pack}')
        self.ctl('c', 'pack', 'add', 'ghost', ghost, 'quic://127.0.0.1:9', expect=REJECTED)
        self.record('a revocation reaches a node through a peer', {'removed': ['ghost']})

        # Addition D: A and B apply different valid admissions as sequence 7;
        # C (at 6) sees both and fences before applying either.
        x = self.statement('admit-x', ['admit', '--after', revoke, '--member', self.member(10)])
        y = self.statement('admit-y', ['admit', '--after', revoke, '--member', self.member(11)])
        self.ctl('a', 'governance', 'apply', x)
        self.ctl('b', 'governance', 'apply', y)
        synced = self.sync('c')['result']
        require(synced['applied'] == [] and synced['stopped']['code'] == 'GOVERNANCE_FORK',
                f'conflict: {synced}')
        status = self.head('c')
        require(status['forked'] and status['sequence'] == 6, f'C after conflict: {status}')
        self.sync('c', expect=REJECTED)
        self.ctl('c', 'governance', 'apply', x, expect=REJECTED)
        self.record('conflicting valid successors fence before either applies',
                    {'sequence': 6, 'forked': True})

        require(all(p.poll() is None for p in self.wolves), 'gate process exited')


if __name__ == '__main__':
    sys.exit(main(SyncGate))
