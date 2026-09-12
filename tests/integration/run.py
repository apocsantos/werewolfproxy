#!/usr/bin/env python3
"""Disposable checkout-only Stage 0 acceptance contract. See README.md."""
import argparse
import hashlib
import json
import os
import pathlib
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import uuid

sys.dont_write_bytecode = True
from targets import LARGE_SHA256, MARKER

ROOT = pathlib.Path(__file__).resolve().parents[2]


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def write_json(path, value):
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), 'w') as out:
        out.write(json.dumps(value, indent=2) + '\n')


def snapshot():
    names = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others',
                                     '--exclude-standard'], cwd=ROOT).split(b'\0')
    return {os.fsdecode(n): digest(ROOT / os.fsdecode(n)) for n in names
            if n and (ROOT / os.fsdecode(n)).is_file()}


def control_request(path, cmd, args=None, timeout=5):
    request_id = str(uuid.uuid4())
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(timeout)
        connection.connect(str(path))
        connection.sendall((json.dumps({'id': request_id, 'cmd': cmd,
                                       'args': args or {}}) + '\n').encode())
        data = bytearray()
        deadline = time.monotonic() + timeout
        while b'\n' not in data:
            require(time.monotonic() < deadline, 'control response deadline exceeded')
            connection.settimeout(max(.001, deadline - time.monotonic()))
            chunk = connection.recv(65536)
            require(chunk, 'control EOF before response')
            data.extend(chunk)
            require(len(data) <= 1024 * 1024, 'oversized control response')
    response = json.loads(data)
    require(isinstance(response, dict) and response.get('id') == request_id,
            'control response ID mismatch')
    require(response.get('ok') is True, f'{cmd}: {response.get("error", response)}')
    require('result' in response, 'control response missing result')
    return response['result']


class Lab:
    def __init__(self, keep=False):
        self.base = pathlib.Path(tempfile.mkdtemp(prefix='wwp-lab-'))
        self.keep = keep
        self.processes = []
        self.handles = []
        self.reservations = {}
        self.ports = {}
        self.rows = []
        self.commands = []
        self.env = {'PATH': '/usr/bin:/bin', 'HOME': str(self.base),
                    'XDG_CONFIG_HOME': str(self.base / 'config'),
                    'XDG_RUNTIME_DIR': str(self.base / 'runtime'),
                    'TMPDIR': str(self.base), 'LANG': 'C.UTF-8',
                    'NO_PROXY': '*', 'no_proxy': '*'}
        (self.base / 'runtime').mkdir(mode=0o700)
        self.binary = ROOT / 'target' / 'stage1-lab' / 'debug' / 'werewolfd'
        self.generation = 0

    def spawn(self, argv, label, env=None):
        handle = (self.base / f'{len(self.commands):03d}-{label}.log').open('wb')
        self.handles.append(handle)
        process = subprocess.Popen([str(x) for x in argv], cwd=ROOT,
                                   env=env or self.env, stdout=handle,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        self.processes.append(process)
        self.commands.append({'label': label, 'argv': [str(x) for x in argv],
                              'pid': process.pid, 'log': handle.name})
        write_json(self.base / 'commands.json', self.commands)
        return process, handle

    def command(self, argv, label, timeout=30, env=None):
        process, handle = self.spawn(argv, label, env)
        try:
            process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.stop(process)
            raise RuntimeError(f'{label} timed out after {timeout}s')
        output = pathlib.Path(handle.name).read_bytes()
        require(process.returncode == 0,
                f'{label} exited {process.returncode}: {output[-4000:].decode(errors="replace")}')
        return output

    @staticmethod
    def stop(process):
        # Kill the entire owned session even if its original parent already exited.
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=5)

    def cleanup(self):
        errors = []
        for process in reversed(self.processes):
            try:
                self.stop(process)
            except Exception as exc:
                errors.append(str(exc))
        for reservation in self.reservations.values():
            reservation.close()
        for handle in self.handles:
            handle.close()
        write_json(self.base / 'cleanup.json',
                   {'processes': [{'pid': p.pid, 'exit': p.returncode} for p in self.processes],
                    'errors': errors})
        if not self.keep:
            shutil.rmtree(self.base)
        require(not errors, f'cleanup failed: {errors}')

    def record(self, name, detail):
        self.rows.append({'test': name, 'status': 'PASS', 'detail': detail})
        write_json(self.base / 'results.json', self.rows)
        print(f'PASS  {name}', flush=True)

    def control(self, wolf, cmd, args=None):
        result = control_request(self.base / f'{wolf}.sock', cmd, args)
        # Pelt initialization returns only fingerprint/path, never private key material.
        with (self.base / 'control.jsonl').open('a') as log:
            log.write(json.dumps({'wolf': wolf, 'cmd': cmd, 'args': args, 'result': result}) + '\n')
        return result

    def reserve(self, key, udp=False):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM if udp else socket.SOCK_STREAM)
        sock.bind(('127.0.0.1', 0))
        self.ports[key] = sock.getsockname()[1]
        self.reservations[key] = sock

    def release(self, key):
        if key in self.reservations:
            self.reservations.pop(key).close()

    def address(self, key):
        return f'127.0.0.1:{self.ports[key]}'

    def wait_for(self, check, description, process=None):
        deadline = time.monotonic() + 15
        last = None
        while time.monotonic() < deadline:
            if process is not None:
                require(process.poll() is None, f'{description}: process exited {process.returncode}')
            try:
                return check()
            except (OSError, ValueError, RuntimeError) as exc:
                last = exc
                time.sleep(.1)
        raise RuntimeError(f'{description}: readiness timeout: {last}')

    def listener(self, key):
        with socket.create_connection(('127.0.0.1', self.ports[key]), timeout=1):
            pass

    def start_wolves(self):
        self.generation += 1
        self.wolves = []
        for wolf in ('a', 'b'):
            den = self.base / wolf
            den.mkdir(mode=0o700, exist_ok=True)
            # The daemon defaults missing Silver state to fail-closed LOCKED.
            # Lab traffic fixtures explicitly opt into OPEN for this run.
            write_json(den / 'silver.json', {'version': 1, 'mode': 'open'})
            os.chmod(den / 'silver.json', 0o600)
            self.release(wolf + '_tcp')
            self.release(wolf + '_quic')
            process, _ = self.spawn([self.binary, '--home', den, '--socket',
                                     self.base / f'{wolf}.sock', '--listen',
                                     self.address(wolf + '_tcp'), '--quic-listen',
                                     self.address(wolf + '_quic')],
                                    f'{wolf}-{self.generation}')
            self.wolves.append(process)
            self.wait_for(lambda: self.control(wolf, 'status'), f'{wolf} control', process)
            self.wait_for(lambda: self.listener(wolf + '_tcp'), f'{wolf} TCP', process)
            exe = pathlib.Path(f'/proc/{process.pid}/exe')
            require(exe.resolve() == self.binary.resolve(), 'daemon is not checkout binary')
            require(digest(exe) == self.binary_hash, 'running executable hash mismatch')

    def stop_wolves(self):
        for process in reversed(self.wolves):
            self.stop(process)

    def url(self, key, path='baseline.txt'):
        return f'http://{self.address(key)}/{path}'

    def get_marker(self, key):
        output = self.command(['curl', '--noproxy', '*', '--fail', '--silent',
                               '--show-error', '--max-time', '12', self.url(key)],
                              f'{key}-application', timeout=15)
        require(output == MARKER, f'{key}: application payload differs')

    def selection(self, expected, key=None, policy="secure", allow_plain=False):
        # Selector stderr contains expected failed-probe diagnostics; capture separately.
        argv = ['bash', ROOT / 'scripts/wolf-b.sh', 'auto', '--policy', policy, '--json']
        if allow_plain:
            argv.append('--allow-plain-fallback')
        error = (self.base / f'select-{len(self.commands)}.stderr').open('wb')
        self.handles.append(error)
        output = self.base / f'select-{len(self.commands)}.stdout'
        with output.open('wb') as out:
            p = subprocess.Popen([str(x) for x in argv], cwd=ROOT, env=self.selector_env,
                                 stdout=out, stderr=error, start_new_session=True)
            self.processes.append(p)
            self.commands.append({'label': 'secure-selector', 'argv': [str(x) for x in argv],
                                  'pid': p.pid})
            write_json(self.base / 'commands.json', self.commands)
            try:
                p.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.stop(p)
                raise RuntimeError('secure selector timed out')
        available = expected != 'unavailable'
        require(p.returncode == (0 if available else 2), f'selector exit {p.returncode}')
        value = json.loads(output.read_bytes())
        require(value.get('schema_version') == 2 and value.get('transport') == expected
                and value.get('healthy') == available and value.get('policy') == policy
                and value.get('fail_closed') == (not available)
                and value.get('security_downgrade') == (expected == 'tcp-plain')
                and value.get('plaintext_authorized') == allow_plain
                and value.get('fallback') == (expected in ('tcp-encrypted-v2', 'tcp-plain'))
                and value.get('url') == (self.url(key) if available else None),
                f'wrong selection: {value}')
        if available:
            self.get_marker(key)
        return value

    def open_profile(self, key):
        self.release(key)
        self.control('b', 'fang.open_profile', {'name': self.profiles[key]['name']})
        self.wait_for(lambda: self.listener(key), f'{key} Fang', self.wolves[1])

    def close(self, key):
        matches = [f for f in self.control('b', 'fang.list') if f['local'] == self.address(key)]
        require(len(matches) == 1, f'{key}: expected exactly one Fang')
        self.control('b', 'fang.close', {'fang_id': matches[0]['id']})
        require(not any(f['local'] == self.address(key) for f in self.control('b', 'fang.list')),
                f'{key}: closed Fang still listed')
        def closed():
            try:
                self.listener(key)
            except ConnectionRefusedError:
                return
            raise RuntimeError('listener still open')
        self.wait_for(closed, f'{key} close')

    def run(self):
        build_env = {k: v for k, v in os.environ.items()
                     if not k.startswith('WEREWOLF_') and k not in ('BASH_ENV', 'ENV')}
        self.command(['cargo', 'build', '--locked', '--offline', '--workspace', '--bins',
                      '--target-dir', ROOT / 'target/stage1-lab'], 'cargo-build',
                     timeout=600, env=build_env)
        self.binary_hash = digest(self.binary)
        provenance = {'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT).decode().strip(),
                      'git_status': subprocess.check_output(['git', 'status', '--short'], cwd=ROOT).decode(),
                      'lock_sha256': digest(ROOT / 'Cargo.lock'),
                      'rustc': self.command(['rustc', '-Vv'], 'rustc', env=build_env).decode(),
                      'cargo': self.command(['cargo', '-V'], 'cargo', env=build_env).decode(),
                      'exercised_daemon': str(self.binary), 'daemon_sha256': self.binary_hash,
                      'cli_built_but_not_exercised': str(self.binary.with_name('werewolfctl')),
                      'selector': str(ROOT / 'scripts/wolf-b.sh'),
                      'selector_sha256': digest(ROOT / 'scripts/wolf-b.sh')}
        write_json(self.base / 'provenance.json', provenance)
        self.provenance = provenance
        # One Pack identity has one address. Reserve both protocol sockets at
        # the same numeric port instead of aliasing an identity in the Pack.
        for wolf in ('a', 'b'):
            for _ in range(128):
                self.reserve(wolf + '_tcp')
                udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                try:
                    udp.bind(('127.0.0.1', self.ports[wolf + '_tcp']))
                except OSError:
                    udp.close()
                    self.release(wolf + '_tcp')
                    continue
                self.ports[wolf + '_quic'] = self.ports[wolf + '_tcp']
                self.reservations[wolf + '_quic'] = udp
                break
            else:
                raise RuntimeError('cannot reserve paired isolated TCP/UDP port')
        for key in ('quic', 'encrypted', 'plain', 'large', 'raw'):
            self.reserve(key)
        target_dir = self.base / 'targets'
        target_dir.mkdir()
        (target_dir / 'baseline.txt').write_bytes(MARKER)
        block = b''.join(hashlib.sha256(i.to_bytes(4, 'big')).digest() for i in range(32768))
        with (target_dir / 'large.bin').open('wb') as out:
            for _ in range(50):
                out.write(block)
        require(digest(target_dir / 'large.bin') == LARGE_SHA256, 'source fixture hash differs from baseline')
        target, _ = self.spawn([sys.executable, pathlib.Path(__file__).with_name('targets.py'),
                                target_dir], 'targets')
        ready = self.wait_for(lambda: json.loads((target_dir / 'ready.json').read_text()),
                              'targets', target)
        self.ports.update(target=ready['http'], echo=ready['echo'])
        self.start_wolves()
        identities = {}
        for wolf in ('a', 'b'):
            self.control(wolf, 'pelt.init')
            pelt = json.loads((self.base / wolf / 'pelt.json').read_text())
            identities[wolf] = {k: pelt[k] for k in ('fingerprint', 'public_key_b64')}
        require(identities['a']['fingerprint'] != identities['b']['fingerprint'], 'identities are not distinct')
        self.stop_wolves()
        def peer(name, wolf, protocol):
            return dict(name=name, trust='Packmate', **identities[wolf],
                        address=f'{protocol}://{self.address(wolf + ("_quic" if protocol == "quic" else "_tcp"))}')
        packs = {'a': [peer('wolf-b', 'b', 'quic')],
                 'b': [peer('wolf-a', 'a', 'quic')]}
        for wolf, pack in packs.items():
            write_json(self.base / wolf / 'pack.json', pack)
        write_json(self.base / 'a' / 'target_policy.json', {
            'mode': 'deny-by-default',
            'peers': {identities['b']['fingerprint']: {'targets': [
                {'address': '127.0.0.1', 'port': self.ports['target']},
                {'address': '127.0.0.1', 'port': self.ports['echo']},
            ]}},
        })
        write_json(self.base / 'b' / 'target_policy.json', {
            'mode': 'deny-by-default', 'peers': {}
        })
        self.start_wolves()
        for wolf in ('a', 'b'):
            require(self.control(wolf, 'pack.list') == packs[wolf], f'{wolf}: Pack load mismatch')
        self.profiles = {}
        for key, name, transport in [('large', 'large-transfer-tcp-v2', 'tcp'),
                                     ('quic', 'home-web-quic', 'quic'),
                                     ('encrypted', 'home-web-tcp-encrypted-v2', 'tcp'),
                                     ('plain', 'home-web-tcp', 'tcp-plain')]:
            profile = dict(name=name, peer='wolf-a',
                           local=self.address(key), remote=self.address('target'), transport=transport)
            self.profiles[key] = profile
            self.control('b', 'fang.profile.add', profile)
            self.open_profile(key)
        write_json(self.base / 'fixtures-public.json',
                   {'ports': self.ports, 'identities': identities, 'packs': packs,
                    'profiles': self.profiles, 'large_sha256': LARGE_SHA256})
        self.selector_env = dict(self.env, WEREWOLF_TARGET_URL=self.url('target'),
                                 WEREWOLF_QUIC_URL=self.url('quic'),
                                 WEREWOLF_TCP_ENC_URL=self.url('encrypted'),
                                 WEREWOLF_TCP_URL=self.url('plain'))
        write_json(self.base / 'selector-environment.json', self.selector_env)
        for key in ('quic', 'encrypted', 'plain'):
            self.get_marker(key)
            self.record(f'{key} application traffic', {'bytes': len(MARKER), 'exact_payload': True})
        policies = [('secure', False), ('compatibility', False),
                    ('compatibility', True), ('legacy', False)]
        selections = {(p, a): [self.selection('quic', 'quic', p, a)] for p, a in policies}
        self.close('quic')
        for p, a in policies:
            selections[p, a].append(self.selection('tcp-encrypted-v2', 'encrypted', p, a))
        self.close('encrypted')
        for p, a in policies:
            plain = p == 'legacy' or a
            selections[p, a].append(self.selection('tcp-plain' if plain else 'unavailable',
                                                   'plain' if plain else None, p, a))
            self.record(f'{p} allow_plain={a} fallback matrix', selections[p, a])
        self.record('unknown policy fails closed', self.selection('unavailable', policy='unknown'))
        self.open_profile('encrypted')
        self.open_profile('quic')
        self.record('restoration to QUIC', [self.selection('quic', 'quic', p, a) for p, a in policies])
        download = self.base / 'download.bin'
        self.command(['curl', '--noproxy', '*', '--fail', '--silent', '--show-error',
                      '--max-time', '180', '--output', download, self.url('large', 'large.bin')],
                     'encrypted-50mib', timeout=185)
        require(download.stat().st_size == 52428800 and digest(download) == LARGE_SHA256,
                '50 MiB encrypted TCP size/SHA256 mismatch')
        self.record('encrypted TCP 50 MiB SHA256', {'bytes': download.stat().st_size,
                                                  'sha256': digest(download)})
        # Extra raw binary application check, transient Fang closed before persistence assertions.
        self.release('raw')
        for transport in ('quic', 'tcp', 'tcp-plain'):
            self.control('b', 'fang.open', dict(peer='wolf-a',
                                              local=self.address('raw'), remote=self.address('echo'),
                                              transport=transport))
            self.wait_for(lambda: self.listener('raw'), 'raw Fang', self.wolves[1])
            payload = bytes(range(256)) * 64
            with socket.create_connection(('127.0.0.1', self.ports['raw']), timeout=12) as connection:
                connection.sendall(payload)
                received = bytearray()
                deadline = time.monotonic() + 12
                while len(received) < len(payload):
                    require(time.monotonic() < deadline, 'raw TCP echo deadline exceeded')
                    connection.settimeout(max(.001, deadline - time.monotonic()))
                    chunk = connection.recv(len(payload) - len(received))
                    require(chunk, 'raw TCP echo EOF')
                    received.extend(chunk)
                require(received == payload, f'{transport}: raw TCP payload differs')
            self.close('raw')
        self.record('raw TCP target across all transports', {'bytes_each': len(payload)})
        before = self.control('b', 'fang.list')
        active_path = self.base / 'b' / 'active_fangs.json'
        active_before = json.loads(active_path.read_text())
        saved_before = {wolf: {name: digest(self.base / wolf / name)
                              for name in ('pelt.json', 'pack.json')}
                        for wolf in ('a', 'b')}
        self.stop_wolves()
        self.start_wolves()
        for key in self.profiles:
            self.wait_for(lambda key=key: self.listener(key), f'restored {key}', self.wolves[1])
        after = self.control('b', 'fang.list')
        require(len(after) == 4 and {f['local'] for f in after} ==
                {p['local'] for p in self.profiles.values()}, 'restored Fang set differs')
        require({f['id'] for f in before}.isdisjoint({f['id'] for f in after}), 'Fang IDs did not change')
        require(active_before == json.loads(active_path.read_text()), 'active profiles changed at restart')
        restored_profiles = self.control('b', 'fang.profile.list')
        require(sorted(restored_profiles, key=lambda p: p['name']) ==
                sorted(self.profiles.values(), key=lambda p: p['name']), 'saved profiles differ')
        for fang in after:
            profile = next(p for p in self.profiles.values() if p['local'] == fang['local'])
            require(all(fang[field] == profile[field] for field in ('peer', 'remote', 'transport'))
                    and fang['state'] == 'Active', 'restored Fang configuration differs')
        for wolf, files in saved_before.items():
            for name, sha in files.items():
                require(digest(self.base / wolf / name) == sha, f'{wolf}/{name} changed at restart')
        for key in ('quic', 'encrypted', 'plain'):
            self.get_marker(key)
        self.command(['curl', '--noproxy', '*', '--fail', '--silent', '--show-error', '--head',
                      '--max-time', '12', self.url('large', 'large.bin')], 'restart-large-head', timeout=15)
        selection = self.selection('quic', 'quic')
        self.record('daemon restart/profile restoration',
                    {'before': before, 'after': after, 'active_profiles': active_before,
                     'selection': selection})
        require(all(p.poll() is None for p in [target, *self.wolves]), 'long-lived lab process exited')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--keep-temp', action='store_true', help='retain private Dens, logs and payloads')
    args = parser.parse_args()
    os.umask(0o077)
    before = snapshot()
    lab = Lab(args.keep_temp)
    print(f'Lab: {lab.base}', flush=True)
    def interrupted(signum, frame):
        raise RuntimeError(f'interrupted by signal {signum}')
    old_handlers = {s: signal.signal(s, interrupted) for s in (signal.SIGINT, signal.SIGTERM)}
    failed = False
    try:
        lab.run()
    except Exception as exc:
        failed = True
        lab.rows.append({'test': 'acceptance completion', 'status': 'FAIL', 'detail': str(exc)})
        write_json(lab.base / 'results.json', lab.rows)
        print(f'FAIL  {exc}', file=sys.stderr, flush=True)
        for path in sorted(lab.base.glob('*.log'))[-4:]:
            print(f'--- {path.name} ---\n{path.read_bytes()[-3000:].decode(errors="replace")}',
                  file=sys.stderr)
    finally:
        for s in old_handlers:
            signal.signal(s, signal.SIG_IGN)
        try:
            lab.cleanup()
        except Exception as exc:
            failed = True
            print(f'FAIL cleanup: {exc}', file=sys.stderr)
        try:
            require(snapshot() == before, 'repository files changed during lab execution')
        except Exception as exc:
            failed = True
            print(f'FAIL source preservation: {exc}', file=sys.stderr)
        for s, handler in old_handlers.items():
            signal.signal(s, handler)
    print(json.dumps({'status': 'FAIL' if failed else 'PASS', 'tests': lab.rows,
                      'provenance': getattr(lab, 'provenance', None),
                      'temporary_state': str(lab.base) if args.keep_temp else 'removed'}), flush=True)
    return int(failed)


if __name__ == '__main__':
    sys.exit(main())
