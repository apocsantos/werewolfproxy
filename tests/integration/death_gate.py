#!/usr/bin/env python3
"""Disposable checkout-only Stage30 death gate. See README.md.

Builds werewolfd with the `lab-faults` feature into its own target directory
(the release and other gates never see it) and checks, on disposable
daemons only:

- death applied from a file on several nodes: exit 67, a Den holding exactly
  the Tombstone and the inert Den lock, no pre-death value in any remaining
  file, former peers unable to reach the node;
- a node offline during the death: `governance sync` finds no death (every
  holder of the statement is dead), the file brings it;
- a dead Den on restart: exit 67 at once, no listener, `den init` refused;
- local retirement: wrong confirmation refused, then the same assertions;
- a storage fault during destruction: incomplete, Pelt gone, completed once
  storage allows;
- crash convergence: an abrupt abort at every step boundary of death and
  retirement converges to a complete Tombstone at the next start.

Not run here: a full disk (needs a size-limited filesystem) and the media
residue measurement (needs root); see README.md.
"""
import base64
import json
import os
import pathlib
import resource
import signal
import socket
import subprocess
import sys
import time

sys.dont_write_bytecode = True
from run import ROOT, digest, main, require, write_json
from service_gate import REJECTED, ServiceGate

TOMBSTONE_EXIT = 67
DEATH_POINTS = ('after_commit', 'after_tombstone', 'after_pelt', 'after_security_state',
                'before_complete')
RETIRE_POINTS = ('after_tombstone', 'after_pelt', 'after_security_state', 'before_complete')
CLEAN = ['.den.lock', 'tombstone.json']


def no_core():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


class DeathGate(ServiceGate):
    WOLVES = ('a', 'b', 'c', 'd', 'x')

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
            'exercised_daemon': str(self.binary), 'daemon_sha256': self.binary_hash})

    def den(self, name):
        return self.base / 'dens' / name

    def start(self, wolf, name=None, halt=None, expect_running=True):
        """Starts one daemon for Den `name` (default: the wolf's) on the wolf's
        ports. With `halt`, the daemon aborts at that step boundary."""
        den = self.den(name or wolf)
        fresh = not den.exists()
        den.mkdir(mode=0o700, parents=True, exist_ok=True)
        if fresh:
            write_json(den / 'silver.json', {'version': 1, 'mode': 'open'})
            os.chmod(den / 'silver.json', 0o600)
        self.release(wolf + '_tcp')
        self.release(wolf + '_quic')
        env = dict(self.env)
        if halt:
            env['WEREWOLF_LAB_TERMINAL_HALT'] = halt
        self.generation += 1
        log = (self.base / f'{self.generation:03d}-{name or wolf}.log').open('wb')
        self.handles.append(log)
        process = subprocess.Popen(
            [str(self.binary), '--home', str(den), '--socket', str(self.base / f'{wolf}.sock'),
             '--listen', self.address(wolf + '_tcp'),
             '--quic-listen', self.address(wolf + '_quic')],
            cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT,
            start_new_session=True, preexec_fn=no_core)
        self.processes.append(process)
        if expect_running:
            self.wait_for(lambda: self.control(wolf, 'status'), f'{wolf} control', process)
        return process

    def exited(self, process, timeout=20):
        try:
            return process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            raise RuntimeError('daemon did not exit')

    def names(self, name):
        return sorted(p.name for p in self.den(name).iterdir())

    def tombstone(self, name):
        return json.loads((self.den(name) / 'tombstone.json').read_text())

    def clean(self, name, kind):
        names = self.names(name)
        require(names == CLEAN, f'{name}: Den is {names}')
        require((self.den(name) / '.den.lock').stat().st_size == 0, f'{name}: lock not empty')
        stone = self.tombstone(name)
        require(stone['kind'] == kind and stone['destruction'] == 'complete',
                f'{name}: tombstone {stone}')
        return stone

    def snapshot(self, wolf):
        """Every value a dead Den must not keep: the Pelt's strings and keys,
        fingerprints, peer names and addresses, in raw, hex and base64."""
        values = set()
        pelt = json.loads((self.den(wolf) / 'pelt.json').read_text())
        for value in pelt.values():
            if isinstance(value, str) and len(value) >= 16:
                values.add(value.encode())
                try:
                    raw = base64.b64decode(value, validate=True)
                    values |= {raw, raw.hex().encode()}
                except ValueError:
                    pass
        for peer in self.ctl(wolf, 'pack', 'list')['result']:
            values |= {peer['name'].encode(), peer['fingerprint'].encode(),
                       peer['address'].split('://')[-1].encode()}
            raw = base64.b64decode(peer['public_key_b64'])
            values |= {peer['public_key_b64'].encode(), raw, raw.hex().encode()}
        return values

    def scan(self, name, needles):
        for path in self.den(name).iterdir():
            data = path.read_bytes()
            found = [n for n in needles if n and n in data]
            require(not found, f'{name}/{path.name} keeps {len(found)} pre-death value(s)')

    def home_ctl(self, name, *args, expect=0):
        result = subprocess.run([str(self.control_binary), '--json', '--home',
                                 str(self.den(name)), *args],
                                cwd=self.base, env=self.env, capture_output=True, text=True,
                                timeout=30)
        require(result.returncode == expect,
                f'{name} {args}: exit {result.returncode}: {result.stderr.strip()}')
        return result

    def no_listener(self, wolf):
        try:
            with socket.create_connection(('127.0.0.1', self.ports[wolf + '_tcp']), timeout=1):
                return False
        except OSError:
            return True

    # --- governance ---------------------------------------------------------

    def gov(self, *args, expect=0):
        return self.home_ctl('gov-tool', 'governance', *[str(a) for a in args], expect=expect)

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

    def fresh_node(self, wolf, name, genesis=True, halt=None):
        """A new node: Pelt, manifest, and (optionally) the genesis."""
        process = self.start(wolf, name, halt=halt)
        self.ctl(wolf, 'pelt', 'init')
        self.ctl(wolf, 'state', 'manifest-migrate')
        if genesis:
            self.ctl(wolf, 'governance', 'apply', self.genesis, '--expect-hash', self.genesis_hash)
        return process

    def apply_death(self, wolf, process):
        result = self.ctl(wolf, 'governance', 'apply', self.death)['result']
        require(result['lifecycle'] == 'dead' and result['destruction'] == 'complete',
                f'{wolf} death: {result}')
        require(self.exited(process) == TOMBSTONE_EXIT, f'{wolf} exit {process.returncode}')
        return result

    # --- the gate -----------------------------------------------------------

    def run(self):
        self.prepare_binaries()
        self.reserve_wolf_ports()
        self.reserve('echo')
        (self.base / 'dens').mkdir(mode=0o700)
        self.den('gov-tool').mkdir(mode=0o700)
        target_dir = self.base / 'targets'
        target_dir.mkdir()
        target, _ = self.spawn([sys.executable, pathlib.Path(__file__).with_name('targets.py'),
                                target_dir], 'targets')
        ready = self.wait_for(lambda: json.loads((target_dir / 'ready.json').read_text()),
                              'targets', target)
        echo_target = f'127.0.0.1:{ready["echo"]}'

        self.gdir = self.base / 'governance'
        self.gdir.mkdir(mode=0o700)
        publics = [json.loads(self.gov('keygen', self.gdir / f'k{i}.key').stdout)['result']
                   ['public_key_b64'] for i in range(10)]
        self.genesis = self.statement('genesis', ['genesis', *[x for p in publics for x in ('--key', p)]])
        self.genesis_hash = json.loads(self.gov('inspect', self.genesis).stdout)['result']['statement']['hash']
        self.death = self.statement('death', ['death', '--after', self.genesis])
        governance_keys = set()
        for p in publics:
            raw = base64.b64decode(p)
            governance_keys |= {p.encode(), raw, raw.hex().encode()}

        # Four live nodes: A trusts B, C and D; each trusts A; D also trusts B
        # and C. B serves an echo target to A.
        processes, keys = {}, {}
        for wolf in 'abcd':
            processes[wolf] = self.fresh_node(wolf, wolf)
            keys[wolf] = self.ctl(wolf, 'pelt', 'show')['result']['public_key_b64']
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'), f'{wolf} TCP')
        links = [('a', 'b'), ('a', 'c'), ('a', 'd'), ('d', 'b'), ('d', 'c')]
        for one, other in links:
            for x, y in ((one, other), (other, one)):
                self.ctl(x, 'pack', 'add', f'wolf-{y}', keys[y], f'quic://{self.address(y + "_quic")}')
        self.ctl('b', 'target', 'allow', 'wolf-a', echo_target)
        self.ctl('a', 'service', 'add', 'echo', 'wolf-b', echo_target, 'tcp')
        self.release('echo')
        self.ctl('a', 'service', 'connect', 'echo', '--local-port', self.ports['echo'])
        self.wait_for(lambda: self.listener('echo'), 'echo listener')
        self.duplex('echo', total=1 << 20)
        needles = set(governance_keys)
        for wolf in 'abcd':
            needles |= self.snapshot(wolf)

        # D goes offline before the death.
        self.stop(processes['d'])

        # Death from a file on B and C.
        for wolf in 'bc':
            result = self.apply_death(wolf, processes[wolf])
            stone = self.clean(wolf, 'death')
            require(stone['terminal_sequence'] == 1 and stone['terminal_hash'] == result['hash'],
                    f'{wolf} tombstone: {stone}')
            self.scan(wolf, needles)
        try:
            self.duplex('echo', total=1 << 16)
            reached = True
        except Exception:
            reached = False
        require(not reached, 'A still reached the dead node B')
        peers = {p['peer']: p['status'] for p in
                 self.ctl('a', 'governance', 'sync')['result']['peers']}
        require(peers['wolf-b'] != 'ok' and peers['wolf-c'] != 'ok', f'sync from dead: {peers}')
        self.record('death from a file leaves only the Tombstone and the inert lock',
                    {'nodes': ['b', 'c'], 'den': CLEAN, 'exit': TOMBSTONE_EXIT,
                     'scanned_values': len(needles), 'a_to_dead_peers': peers})

        # D comes back: sync finds no death (every holder is dead); the file
        # brings it.
        processes['d'] = self.start('d')
        synced = self.ctl('d', 'governance', 'sync')['result']
        require(synced['applied'] == [], f'D sync: {synced}')
        self.apply_death('d', processes['d'])
        self.clean('d', 'death')
        self.scan('d', needles)
        self.record('an offline node learns of death from the file; sync finds none',
                    {'sync_peers': synced['peers']})

        # A dead Den on restart: exit 67 at once, no listener, never re-initialized.
        restarted = self.start('b', expect_running=False)
        require(self.exited(restarted) == TOMBSTONE_EXIT, f'restart exit {restarted.returncode}')
        require(self.no_listener('b'), 'a dead node bound a listener')
        require(self.names('b') == CLEAN, f'restart changed the Den: {self.names("b")}')
        self.home_ctl('b', 'den', 'init', expect=REJECTED)
        self.record('a dead Den exits 67 at start and is never initialized again', {})

        # Retirement of A.
        self.ctl('a', 'den', 'retire', '--confirm', 'wwp1:00', expect=REJECTED)
        fingerprint = self.ctl('a', 'pelt', 'show')['result']['fingerprint']
        retired = self.ctl('a', 'den', 'retire', '--confirm', fingerprint)['result']
        require(retired == {'status': 'retired', 'destruction': 'complete'}, f'retire: {retired}')
        require(self.exited(processes['a']) == TOMBSTONE_EXIT, 'A did not exit 67')
        stone = self.clean('a', 'retirement')
        require(set(stone) == {'format', 'version', 'kind', 'destruction', 'software'},
                f'retirement tombstone fields: {sorted(stone)}')
        self.scan('a', needles)
        self.record('local retirement: confirmation, then only the Tombstone', {})

        # A storage fault during destruction: an entry that cannot be removed.
        process = self.fresh_node('x', 'fault', genesis=False)
        fp = self.ctl('x', 'pelt', 'show')['result']['fingerprint']
        stuck = self.den('fault') / 'stuck'
        stuck.mkdir(mode=0o700)
        (stuck / 'file').write_bytes(b'x')
        os.chmod(stuck, 0o500)
        result = self.ctl('x', 'den', 'retire', '--confirm', fp)['result']
        require(result['destruction'] == 'incomplete', f'fault: {result}')
        require(self.exited(process) == TOMBSTONE_EXIT, 'fault node did not exit 67')
        require('pelt.json' not in self.names('fault'), 'the Pelt survived a failed destruction')
        require(self.tombstone('fault')['destruction'] == 'incomplete', 'claimed complete')
        again = self.start('x', 'fault', expect_running=False)
        require(self.exited(again) == TOMBSTONE_EXIT, 'faulted restart')
        require(self.tombstone('fault')['destruction'] == 'incomplete', 'complete while stuck')
        os.chmod(stuck, 0o700)
        again = self.start('x', 'fault', expect_running=False)
        require(self.exited(again) == TOMBSTONE_EXIT, 'repaired restart')
        self.clean('fault', 'retirement')
        self.record('a storage fault leaves incomplete, Pelt gone; storage back, complete', {})

        # Crash convergence at every step boundary.
        crashes = {}
        for kind, points in (('death', DEATH_POINTS), ('retirement', RETIRE_POINTS)):
            for point in points:
                name = f'crash-{kind}-{point}'
                process = self.fresh_node('x', name, genesis=(kind == 'death'), halt=point)
                if kind == 'death':
                    self.ctl('x', 'governance', 'apply', self.death, expect=None)
                else:
                    fp = self.ctl('x', 'pelt', 'show')['result']['fingerprint']
                    self.ctl('x', 'den', 'retire', '--confirm', fp, expect=None)
                code = self.exited(process)
                require(code == -signal.SIGABRT, f'{name}: expected an abort, got {code}')
                restarted = self.start('x', name, expect_running=False)
                require(self.exited(restarted) == TOMBSTONE_EXIT, f'{name}: restart')
                self.clean(name, kind)
                crashes[name] = 'converged'
        self.record('an abort at every step boundary converges at the next start', crashes)
        require(target.poll() is None, 'target process exited')

    def ctl(self, wolf, *args, expect=0):
        if expect is not None:
            return super().ctl(wolf, *args, expect=expect)
        argv = [str(self.control_binary), '--json', '--socket', str(self.base / f'{wolf}.sock'),
                *[str(a) for a in args]]
        subprocess.run(argv, cwd=self.base, env=self.env, capture_output=True, timeout=30)
        return {}


if __name__ == '__main__':
    sys.exit(main(DeathGate))
