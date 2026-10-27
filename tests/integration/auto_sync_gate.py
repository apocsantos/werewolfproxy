#!/usr/bin/env python3
"""Disposable checkout-only Stage31 automatic sync gate. See README.md.

Builds werewolfd with the `lab-faults` feature (for exact lab delays) into
its own target directory and checks, on disposable daemons only:

- ceremony nodes never sync on their own;
- after activation every active, admitted node runs on its own;
- a revocation applied by file on one node reaches the others with no
  operator command, closing the revoked member's Fang; archives stay
  byte-identical; nothing is applied twice;
- an unadmitted node never syncs, and starts on its own once its Admission
  arrives by file;
- the switch survives a restart and stops all automatic RPCs, counted at a
  UDP sink standing in for a Pack member (the responder's side);
- an operator `governance sync` during a resolving automatic run wins;
- Silver stops it and clearing Silver restarts it; a fork found by an
  automatic run fences the node and stops further runs;
- no responder ever rejects an automatic run (its token bucket holds).

DYING is covered by the unit tests and the death gate.
"""
import json
import os
import pathlib
import socket
import subprocess
import sys
import time

sys.dont_write_bytecode = True
from run import ROOT, digest, main, require, write_json
from service_gate import REJECTED, ServiceGate

FIRST_MS = 300
INTERVAL_MS = 1500
INTERVAL = INTERVAL_MS / 1000
REAL = ('wolf-a', 'wolf-b', 'wolf-c', 'wolf-d', 'wolf-u')


class AutoSyncGate(ServiceGate):
    WOLVES = ('a', 'b', 'c', 'd', 'u')

    # --- harness ------------------------------------------------------------

    def prepare_binaries(self):
        build_env = {k: v for k, v in os.environ.items()
                     if not k.startswith('WEREWOLF_') and k not in ('BASH_ENV', 'ENV')}
        target = ROOT / 'target/stage30-lab-faults'
        self.command(['cargo', 'build', '--locked', '--offline', '--workspace', '--bins',
                      '--features', 'werewolfd/lab-faults', '--target-dir', target],
                     'cargo-build-lab-faults', timeout=900, env=build_env)
        self.binary = target / 'debug' / 'werewolfd'
        self.control_binary = target / 'debug' / 'werewolfctl'
        self.binary_hash = digest(self.binary)
        write_json(self.base / 'provenance.json', {
            'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT).decode().strip(),
            'git_status': subprocess.check_output(['git', 'status', '--short'], cwd=ROOT).decode(),
            'features': ['werewolfd/lab-faults'],
            'lab_delays_ms': {'first': FIRST_MS, 'interval': INTERVAL_MS},
            'exercised_daemon': str(self.binary), 'daemon_sha256': self.binary_hash})

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

    def apply(self, wolves, statement, *extra):
        return {w: self.ctl(w, 'governance', 'apply', statement, *extra)['result']
                for w in wolves}

    def status(self, wolf):
        return self.ctl(wolf, 'governance', 'status')['result']

    def auto(self, wolf):
        return self.status(wolf)['auto_sync']

    def until(self, check, description, timeout=20):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = check()
            if value:
                return value
            time.sleep(.1)
        raise RuntimeError(f'{description}: not reached in {timeout}s')

    def run_after(self, wolf, since):
        """The wolf's first automatic run that started after `since`."""
        def check():
            auto = self.auto(wolf)
            run = auto['last_run']
            if run and time.monotonic() - run['started_secs_ago'] >= since - 1:
                self.audit(wolf, run)
                return run
            return None
        return check

    def audit(self, wolf, run):
        """No real responder ever rejects an automatic run."""
        for peer in run['peers']:
            if peer['peer'] in REAL:
                require(peer['status'] == 'ok', f'{wolf}: {peer} in {run}')

    def applied(self, wolf):
        """Every statement the wolf applied automatically, from its logs."""
        marker = '[AUTO_SYNC_APPLIED] '
        found = []
        for path in sorted(self.base.glob(f'*-{wolf}-*.log')):
            for line in path.read_text(errors='replace').splitlines():
                if marker in line:
                    found.append(json.loads(line.split(marker, 1)[1]))
        return found

    def applied_once(self, wolf, sequence):
        found = [r for r in self.applied(wolf) if r.get('sequence') == sequence]
        return found[0] if len(found) == 1 else None

    def drain(self):
        """Datagrams the UDP sink (wolf-trap) received since the last drain."""
        sink = self.reservations['trap']
        sink.setblocking(False)
        count = 0
        while True:
            try:
                sink.recv(65536)
                count += 1
            except BlockingIOError:
                return count

    # --- the gate -----------------------------------------------------------

    def run(self):
        self.prepare_binaries()
        self.env = dict(self.env, WEREWOLF_LAB_AUTO_SYNC_FIRST_MS=str(FIRST_MS),
                        WEREWOLF_LAB_AUTO_SYNC_INTERVAL_MS=str(INTERVAL_MS))
        self.reserve_wolf_ports()
        self.reserve('fang_bd')
        self.reserve('trap', udp=True)
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

        self.gdir = self.base / 'governance'
        self.gdir.mkdir(mode=0o700)
        publics = [self.gov('keygen', self.gdir / f'k{i}.key')['result']['public_key_b64']
                   for i in range(10)]
        trap_key = self.gov('keygen', self.gdir / 'trap.key')['result']['public_key_b64']
        dummy_key = self.gov('keygen', self.gdir / 'dummy.key')['result']['public_key_b64']
        genesis = self.statement('genesis', ['genesis', *[x for p in publics for x in ('--key', p)]])
        genesis_hash = self.gov('inspect', genesis)['result']['statement']['hash']
        self.apply(self.WOLVES, genesis, '--expect-hash', genesis_hash)
        # A, B, C and D know each other; U knows A and A knows U.
        for wolf in ('a', 'b', 'c', 'd'):
            for peer in ('a', 'b', 'c', 'd'):
                if peer != wolf:
                    self.ctl(wolf, 'pack', 'add', f'wolf-{peer}', keys[peer],
                             f'quic://{self.address(peer + "_quic")}')
        self.ctl('u', 'pack', 'add', 'wolf-a', keys['a'], f'quic://{self.address("a_quic")}')
        self.ctl('a', 'pack', 'add', 'wolf-u', keys['u'], f'quic://{self.address("u_quic")}')

        # 1. The ceremony stays manual.
        admit = self.statement('admit', ['admit', '--after', genesis,
                                         *[x for w in 'abcd' for x in ('--member', keys[w])],
                                         '--member', trap_key])
        self.apply(['a'], admit)
        time.sleep(3 * INTERVAL)
        for wolf in self.WOLVES:
            auto = self.auto(wolf)
            require(auto['ineligible_reason'] == 'not_activated' and auto['last_run'] is None
                    and auto['phase'] == 'ineligible', f'{wolf} in the ceremony: {auto}')
        require([self.status(w)['sequence'] for w in 'bcd'] == [0, 0, 0],
                'a ceremony node synced on its own')
        self.record('ceremony nodes never sync on their own',
                    {'waited_s': 3 * INTERVAL, 'heads_bcd': [0, 0, 0]})

        # 2. Activation by file; every active, admitted node then runs alone.
        self.apply('bcdu', admit)
        activate = self.statement('activate', ['activate', '--after', admit])
        activated = time.monotonic()
        self.apply(self.WOLVES, activate)
        for wolf in 'abcd':
            run = self.until(self.run_after(wolf, activated), f'{wolf} first automatic run')
            require(run['outcome'] == 'nothing_new', f'{wolf}: {run}')
        auto_u = self.auto('u')
        require(auto_u['ineligible_reason'] == 'unadmitted' and auto_u['last_run'] is None,
                f'unadmitted U: {auto_u}')
        self.record('after activation each active, admitted node runs on its own',
                    {'nodes': 'abcd', 'u': 'unadmitted, no run'})

        # 3. A revocation applied by file on A alone reaches B and C.
        self.ctl('d', 'target', 'allow', 'wolf-b', echo_target)
        self.release('fang_bd')
        self.ctl('b', 'fang', 'open', 'wolf-d', self.address('fang_bd'), echo_target)
        require(len(self.active_fangs('b')) == 1, 'B has no Fang to D')
        revoke_d = self.statement('revoke-d', ['revoke', '--after', activate,
                                               '--member', keys['d']])
        self.apply(['a'], revoke_d)
        for wolf in ('b', 'c'):
            self.until(lambda wolf=wolf: self.status(wolf)['sequence'] == 3,
                       f'{wolf} received the revocation')
        run_b = self.until(lambda: self.applied_once('b', 3), 'B automatic apply')
        require(run_b['revoked_members'] == ['wolf-d'] and run_b['closed_fangs'] == 1,
                f'B revocation: {run_b}')
        require(not self.active_fangs('b'), 'B kept a Fang to D')
        # D cannot learn its own revocation by sync: its former peers no
        # longer have it in their Packs and refuse it. The file does it.
        run_d = self.until(
            lambda: (run := self.auto('d')['last_run']) and all(
                p['status'] == 'rejected' for p in run['peers']) and run,
            'D refused by its peers')
        require(self.status('d')['sequence'] == 2, 'D learned its revocation by sync')
        self.apply(['d'], revoke_d)
        auto_d = self.until(lambda: (a := self.auto('d'))['phase'] == 'ineligible' and a,
                            'D ineligible')
        require(auto_d['ineligible_reason'] == 'unadmitted', f'D: {auto_d}')
        file_bytes = revoke_d.read_bytes()
        for wolf in ('b', 'c'):
            require((self.base / wolf / 'governance_00000003.wwgov').read_bytes() == file_bytes,
                    f'{wolf} archive differs from the file')
        self.record('a revocation by file on one node propagates with no command',
                    {'closed_fangs_on_b': 1, 'archives': 'identical',
                     'd_by_sync': [p['status'] for p in run_d['peers']],
                     'd_after_file': 'unadmitted'})

        # 4. U: unadmitted, no runs; its Admission by file starts it.
        admit_u = self.statement('admit-u', ['admit', '--after', revoke_d, '--member', keys['u']])
        self.apply(['a'], admit_u)
        self.ctl('a', 'pack', 'add', 'wolf-u', keys['u'], f'quic://{self.address("u_quic")}')
        require(self.auto('u')['last_run'] is None, 'U ran while unadmitted')
        # U is at sequence 2: it needs the revocation and its Admission, by file.
        admitted = time.monotonic()
        self.apply(['u'], revoke_d)
        self.apply(['u'], admit_u)
        run_u = self.until(self.run_after('u', admitted), 'U first automatic run')
        require(run_u['outcome'] == 'nothing_new'
                and run_u['peers'] == [{'peer': 'wolf-a', 'status': 'ok', 'offered': [0]}],
                f'U run: {run_u}')
        for wolf in ('b', 'c'):
            self.until(lambda wolf=wolf: self.status(wolf)['sequence'] == 4,
                       f'{wolf} received the Admission of U')
        self.record('an unadmitted node starts on its own once admitted by file',
                    {'u_first_run': run_u['outcome']})

        # 5. The switch, at the responder's side: a UDP sink as wolf-trap.
        self.ctl('b', 'pack', 'add', 'wolf-trap', trap_key, f'quic://{self.address("trap")}')
        self.drain()
        time.sleep(3 * INTERVAL + 8)
        on = self.drain()
        require(on > 0, 'no automatic datagram reached the sink while on')
        self.ctl('b', 'governance', 'auto-sync', 'off')
        self.stop_wolves()
        self.start_wolves()
        auto_b = self.auto('b')
        require(auto_b['enabled'] is False and auto_b['ineligible_reason'] == 'switched_off',
                f'switch after restart: {auto_b}')
        self.drain()
        time.sleep(3 * INTERVAL + 8)
        off = self.drain()
        require(off == 0, f'{off} datagram(s) reached the sink while off')
        self.record('the switch survives a restart and stops all automatic RPCs',
                    {'sink_datagrams_on': on, 'sink_datagrams_off': off})

        # 6. Operator first: a sync during a resolving run (held by the sink).
        self.ctl('b', 'governance', 'auto-sync', 'on')
        self.until(lambda: self.auto('b')['phase'] == 'resolving', 'B resolving', timeout=10)
        started = time.monotonic()
        synced = self.ctl('b', 'governance', 'sync', '--peer', 'wolf-a')['result']
        require(synced['peers'] == [{'peer': 'wolf-a', 'status': 'ok', 'offered': [0]}],
                f'operator sync: {synced}')
        cancelled = self.until(lambda: self.auto('b')['last_run'], 'B cancelled run')
        require(cancelled['outcome'] == 'cancelled', f'B run: {cancelled}')
        self.record('an operator sync wins over a resolving automatic run',
                    {'operator_sync_s': round(time.monotonic() - started, 2),
                     'automatic': 'cancelled'})
        self.ctl('b', 'pack', 'remove', 'wolf-trap')

        # 7. Silver stops it; clearing Silver starts it again.
        self.ctl('c', 'silver', 'on')
        auto_c = self.until(lambda: (a := self.auto('c'))['phase'] == 'ineligible' and a,
                            'C ineligible in Silver')
        require(auto_c['ineligible_reason'] == 'silver', f'C in Silver: {auto_c}')
        cleared = time.monotonic()
        self.ctl('c', 'silver', 'off')
        self.until(self.run_after('c', cleared), 'C run after Silver')
        self.record('Silver stops automatic sync; clearing it starts again', {})

        # 8. A fork found by an automatic run fences B and stops it.
        self.ctl('b', 'governance', 'auto-sync', 'off')
        left = self.statement('left', ['revoke', '--after', admit_u, '--member', trap_key])
        right = self.statement('right', ['admit', '--after', admit_u, '--member', dummy_key])
        self.ctl('c', 'governance', 'auto-sync', 'off')
        self.apply(['a'], left)
        self.apply(['c'], right)
        self.ctl('b', 'governance', 'auto-sync', 'on')
        fork = self.until(lambda: self.auto('b')['last_run']['outcome'] == 'fork'
                          and self.auto('b'), 'B fork')
        require(self.status('b')['forked'] and fork['ineligible_reason'] == 'forked'
                and fork['phase'] == 'ineligible', f'B after fork: {fork}')
        frozen = fork['last_run']['started_secs_ago']
        time.sleep(3 * INTERVAL)
        later = self.auto('b')['last_run']
        require(later['outcome'] == 'fork' and later['started_secs_ago'] >= frozen + 3 * INTERVAL - 1,
                f'B ran again after the fence: {later}')
        self.record('a fork found by an automatic run fences the node and stops it',
                    {'b_sequence': self.status('b')['sequence']})

        # Every statement was applied automatically exactly once per node.
        counts = {}
        for wolf in ('b', 'c'):
            sequences = [r['sequence'] for r in self.applied(wolf)]
            counts[wolf] = sequences
            require(sorted(sequences) == [3, 4], f'{wolf} applied {sequences}')
        self.record('nothing is applied twice', {'automatic_applies': counts})
        require(all(p.poll() is None for p in [target, *self.wolves]), 'gate process exited')


if __name__ == '__main__':
    sys.exit(main(AutoSyncGate))
