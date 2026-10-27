#!/usr/bin/env python3
"""Stage32 robustness and chaos harness (docs/STAGE32_ROBUSTNESS_SPEC.md).

    python3 -B tests/integration/chaos.py dry-run [--keep-temp]

Builds the featureless release certification build, then re-runs itself
inside an unprivileged sandbox (`unshare -rmnp`: user, mount, network and
PID namespaces). Inside, it builds a private root (read-only system binds,
chroot) so the Den ancestor check runs unchanged, one network namespace per
node joined through a router namespace, and drives disposable daemons over
their control sockets. Nothing in the sandbox can see or reach any process
or network outside it; when the sandbox's first process ends, everything in
it ends.

`dry-run` is spec step 2: S1 (encrypted TCP, direct, 32 MiB full duplex)
under P0 and P2, 3 runs each, with the PASS items that apply to it.
"""
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
from run import ROOT, control_request, digest, require, write_json

HERE = pathlib.Path(__file__).resolve().parent
CERT_TARGET = ROOT / 'target/stage32-chaos'
NODES = {'a': '10.32.1', 'b': '10.32.2', 'y': '10.32.3'}
PORT = 8443
SYSTEM_DIRS = ('usr', 'lib', 'lib64', 'bin', 'sbin', 'etc')
SETTLE_S = 35
PROFILES = {
    'P0': None,
    'P1': 'loss 1%',
    'P2': 'loss 10%',
    'P3': 'loss gemodel 5% 15% 75% 0%',
    'P4': 'delay 50ms 20ms distribution normal',
    'P5': 'delay 200ms 100ms distribution normal',
    'P6': 'delay 10ms reorder 25% 50%',
    'P7': 'duplicate 5%',
    'P8': 'corrupt 1%',
    'P9': 'rate 1mbit limit 100',
}
LEAK = {'fds': 4, 'threads': 2, 'rss_kib': 32 * 1024}
MIB = 1 << 20


def sh(*argv, check=True, capture=True):
    out = subprocess.run([str(a) for a in argv], capture_output=capture, text=True)
    if check and out.returncode != 0:
        raise RuntimeError(f'{" ".join(map(str, argv))}: {out.stderr.strip()}')
    return out


# --- outside the sandbox ----------------------------------------------------

def build(target):
    env = {k: v for k, v in os.environ.items()
           if not k.startswith('WEREWOLF_') and k not in ('BASH_ENV', 'ENV')}
    sh('cargo', 'build', '--release', '--locked', '--offline', '--workspace', '--bins',
       '--target-dir', target)
    binaries = target / 'release'
    tool = lambda *a: subprocess.check_output(a, env=env).decode().strip()
    return binaries, {
        'build': 'certification', 'features': [],
        'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT).decode().strip(),
        'git_status': subprocess.check_output(['git', 'status', '--short'], cwd=ROOT).decode(),
        'rustc': tool('rustc', '-V'), 'cargo': tool('cargo', '-V'),
        'werewolfd_sha256': digest(binaries / 'werewolfd'),
        'werewolfctl_sha256': digest(binaries / 'werewolfctl'),
    }


def host_facts():
    faketime = subprocess.run(['dpkg-query', '-W', '-f=${Version}', 'libfaketime'],
                              capture_output=True, text=True).stdout.strip()
    return {'kernel': os.uname().release,
            'iproute2': sh('/usr/sbin/tc', '-V').stdout.strip(),
            'libfaketime': faketime or None}


def outside(args):
    binaries, provenance = build(CERT_TARGET)
    work = pathlib.Path(tempfile.mkdtemp(prefix='wwp-chaos-'))
    os.chmod(work, 0o755)
    write_json(work / 'provenance-certification.json', provenance)
    write_json(work / 'host.json', host_facts())
    status = 1
    try:
        inner = subprocess.run(
            ['unshare', '-rmnp', '--fork', '--kill-child', '--',
             sys.executable, '-B', __file__, '--inside', str(work), str(binaries),
             args.campaign])
        status = inner.returncode
        results = work / 'results.json'
        if results.exists():
            print(results.read_text().strip())
        else:
            print(json.dumps({'status': 'EXCLUDED', 'reason': 'sandbox produced no results',
                              'exit': inner.returncode}))
            status = status or 1
    finally:
        if args.keep_temp or status != 0:
            print(f'kept: {work}', file=sys.stderr)
        else:
            shutil.rmtree(work)
    return status


# --- inside the sandbox -----------------------------------------------------

class Sandbox:
    def __init__(self, work, binaries):
        self.work = work
        self.binaries = binaries
        self.root = work / 'root'
        self.processes = {}
        self.logs = []

    # The private root and the network.

    def build_root(self):
        sh('mount', '-t', 'proc', 'proc', '/proc')
        sh('mount', '-t', 'tmpfs', 'none', '/run')
        os.makedirs('/run/netns')
        self.root.mkdir(mode=0o755)
        for name in SYSTEM_DIRS:
            if not pathlib.Path('/', name).exists():
                continue
            target = self.root / name
            if pathlib.Path('/', name).is_symlink():
                target.symlink_to(os.readlink(pathlib.Path('/', name)))
                continue
            target.mkdir()
            sh('mount', '--rbind', f'/{name}', target)
            sh('mount', '-o', 'remount,bind,ro', target)
        for name in ('proc', 'dev', 'bins', 'tmp'):
            (self.root / name).mkdir()
        sh('mount', '-t', 'proc', 'proc', self.root / 'proc')
        sh('mount', '--rbind', '/dev', self.root / 'dev')
        sh('mount', '--bind', self.binaries, self.root / 'bins')
        sh('mount', '-o', 'remount,bind,ro', self.root / 'bins')
        (self.root / 'work').mkdir(mode=0o700)
        # The Den check sees only namespace-owned ancestors, and the system
        # binds really are read-only.
        owners = {p: os.stat(self.root / p).st_uid for p in ('.', 'work')}
        read_only = {}
        for path in ('/usr/.wwp-ro-probe', '/etc/.wwp-ro-probe', '/bins/.wwp-ro-probe'):
            write = subprocess.run(['/usr/sbin/chroot', self.root, '/bin/touch', path],
                                   capture_output=True, text=True)
            read_only[path] = write.returncode != 0 and 'Read-only' in write.stderr
        return {'root_owner': owners['.'], 'work_owner': owners['work'],
                'read_only': read_only,
                'system_binds_read_only': all(read_only.values())}

    def build_network(self, nodes):
        sh('ip', 'netns', 'add', 'r')
        sh('ip', '-n', 'r', 'link', 'set', 'lo', 'up')
        for node in nodes:
            net = NODES[node]
            sh('ip', 'netns', 'add', node)
            sh('ip', 'link', 'add', f'v{node}', 'type', 'veth', 'peer', 'name', f'r{node}')
            sh('ip', 'link', 'set', f'v{node}', 'netns', node)
            sh('ip', 'link', 'set', f'r{node}', 'netns', 'r')
            sh('ip', '-n', node, 'addr', 'add', f'{net}.2/24', 'dev', f'v{node}')
            sh('ip', '-n', 'r', 'addr', 'add', f'{net}.1/24', 'dev', f'r{node}')
            for ns, dev in ((node, 'lo'), (node, f'v{node}'), ('r', f'r{node}')):
                sh('ip', '-n', ns, 'link', 'set', dev, 'up')
            sh('ip', '-n', node, 'route', 'add', 'default', 'via', f'{net}.1')
        self.in_net('r', 'sh', '-c', 'echo 1 > /proc/sys/net/ipv4/ip_forward')

    def in_net(self, ns, *argv, **kwargs):
        return sh('nsenter', f'--net=/run/netns/{ns}', *argv, **kwargs)

    # Impairment: netem on both ends of a node's link.

    def impair(self, node, profile, seed):
        spec = PROFILES[profile]
        commands = []
        for ns, dev, s in ((node, f'v{node}', seed), ('r', f'r{node}', seed + 1)):
            self.in_net(ns, '/usr/sbin/tc', 'qdisc', 'del', 'dev', dev, 'root', check=False)
            if spec is None:
                continue
            argv = ['/usr/sbin/tc', 'qdisc', 'add', 'dev', dev, 'root', 'netem',
                    *spec.split(), 'seed', str(s)]
            self.in_net(ns, *argv)
            shown = self.in_net(ns, '/usr/sbin/tc', 'qdisc', 'show', 'dev', dev).stdout.strip()
            require('netem' in shown, f'netem not applied on {ns}/{dev}: {shown}')
            commands.append({'ns': ns, 'argv': argv[1:], 'shown': shown})
        return commands

    # Daemons.

    def start(self, node, env=None):
        den = self.root / 'work' / node
        den.mkdir(mode=0o700)
        write_json(den / 'silver.json', {'version': 1, 'mode': 'open'})
        address = f'{NODES[node]}.2:{PORT}'
        log = (self.work / f'{node}.log').open('ab')
        self.logs.append(log)
        process = subprocess.Popen(
            ['nsenter', f'--net=/run/netns/{node}', '/usr/sbin/chroot', self.root,
             '/bins/werewolfd', '--home', f'/work/{node}', '--socket', f'/work/{node}.sock',
             '--listen', address, '--quic-listen', address],
            stdout=log, stderr=subprocess.STDOUT, env=env or {'PATH': '/usr/bin:/bin'})
        self.processes[node] = process
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            try:
                self.ctl(node, 'status')
                return process
            except (OSError, RuntimeError):
                require(process.poll() is None, f'{node} exited {process.returncode}')
                time.sleep(.1)
        raise RuntimeError(f'{node} control not ready')

    def ctl(self, node, cmd, args=None, timeout=10):
        return control_request(self.root / 'work' / f'{node}.sock', cmd, args, timeout)

    def sample(self, node):
        pid = self.processes[node].pid
        status = pathlib.Path(f'/proc/{pid}/status').read_text()
        field = lambda name: int(next(l.split()[1] for l in status.splitlines()
                                      if l.startswith(name + ':')))
        return {'fds': len(os.listdir(f'/proc/{pid}/fd')), 'threads': field('Threads'),
                'rss_kib': field('VmRSS'), 'alive': self.processes[node].poll() is None}

    def targets(self, node):
        directory = self.work / f'targets-{node}'
        directory.mkdir()
        log = (self.work / f'targets-{node}.log').open('ab')
        self.logs.append(log)
        subprocess.Popen(['nsenter', f'--net=/run/netns/{node}', sys.executable, '-I', '-B',
                          HERE / 'targets.py', directory], stdout=log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 15
        while not (directory / 'ready.json').exists():
            require(time.monotonic() < deadline, 'targets not ready')
            time.sleep(.1)
        return json.loads((directory / 'ready.json').read_text())

    def client(self, node, address, size, label, limit):
        out = self.in_net(node, sys.executable, '-I', '-B', HERE / 'chaos_client.py',
                          'duplex', address, size, label, limit, check=False)
        lines = out.stdout.strip().splitlines()
        if out.returncode != 0 or not lines:
            return {'error': f'client exit {out.returncode}: {out.stderr.strip()[-300:]}',
                    'harness': True}
        return json.loads(lines[-1])


def judge_leak(baseline, after):
    over = {k: after[k] - baseline[k] for k in LEAK if after[k] - baseline[k] > LEAK[k]}
    return over


def dry_run(box):
    result = {'campaign': 'dry-run', 'spec_step': 2, 'surface': 'S1',
              'impaired_link': 'A-R (both ends)'}
    result['sandbox'] = box.build_root()
    require(result['sandbox']['root_owner'] == 0 and result['sandbox']['work_owner'] == 0,
            f'private root not namespace-owned: {result["sandbox"]}')
    require(result['sandbox']['system_binds_read_only'], 'system binds are writable')
    box.build_network(('a', 'b'))
    echo = box.targets('b')['echo']
    for node in ('a', 'b'):
        box.start(node)
        box.ctl(node, 'pelt.init')
        box.ctl(node, 'state.manifest.migrate')
    ids = {n: box.ctl(n, 'pelt.fingerprint') for n in ('a', 'b')}
    for node, peer in (('a', 'b'), ('b', 'a')):
        box.ctl(node, 'pack.add', {'name': f'wolf-{peer}',
                                   'fingerprint': ids[peer]['fingerprint'],
                                   'public_key_b64': ids[peer]['public_key_b64'],
                                   'address': f'tcp://{NODES[peer]}.2:{PORT}'})
    box.ctl('b', 'target.allow', {'peer': 'wolf-a', 'target': f'127.0.0.1:{echo}'})
    time.sleep(2)
    baseline = {n: box.sample(n) for n in ('a', 'b')}
    result['baseline'] = baseline

    runs = []
    p0_times = []
    size = 32 * MIB
    for profile in ('P0', 'P2'):
        k = int(profile[1:])
        for n in (1, 2, 3):
            seed = 1000 * k + 10 * n
            run = {'profile': profile, 'run': n, 'seed': seed, 'build': 'certification'}
            run['netem'] = box.impair('a', profile, seed)
            bound = 120 if profile == 'P0' else max(120, 10 * max(p0_times))
            run['outer_bound_s'] = bound
            opened = box.ctl('a', 'fang.open', {'peer': 'wolf-b', 'local': '127.0.0.1:17001',
                                                 'remote': f'127.0.0.1:{echo}',
                                                 'transport': 'tcp'}, timeout=30)
            run['fang'] = {k2: opened.get(k2) for k2 in ('transport', 'status') if k2 in opened}
            transfer = box.client('a', '127.0.0.1:17001', size, f'{profile}-{n}', bound)
            run['transfer'] = transfer
            for fang in box.ctl('a', 'fang.list'):
                box.ctl('a', 'fang.close', {'fang_id': fang['id']})
            box.impair('a', 'P0', seed)
            time.sleep(SETTLE_S)
            after = {node: box.sample(node) for node in ('a', 'b')}
            run['after_settle'] = after
            reasons = []
            if transfer.get('harness'):
                run['result'], run['reason'] = 'EXCLUDED', transfer['error']
            else:
                if transfer.get('complete') and not transfer.get('match'):
                    reasons.append('integrity: SHA-256 mismatch')
                if not transfer.get('complete') and transfer.get('error') == 'outer bound exceeded':
                    reasons.append('liveness: outer bound exceeded')
                if not transfer.get('complete') and profile == 'P0':
                    reasons.append(f'baseline transfer failed: {transfer.get("error")}')
                for node in ('a', 'b'):
                    if not after[node]['alive']:
                        reasons.append(f'{node} exited')
                    over = judge_leak(baseline[node], after[node])
                    if over:
                        reasons.append(f'{node} over leak margin: {over}')
                run['result'] = 'FAIL' if reasons else 'PASS'
                run['reason'] = '; '.join(reasons) or None
            if profile == 'P0' and transfer.get('complete'):
                p0_times.append(transfer['elapsed_s'])
            runs.append(run)
            print(f'{run["result"]:8} {profile} run {n}: '
                  f'{transfer.get("elapsed_s")} s, match={transfer.get("match")}, '
                  f'error={transfer.get("error")}', file=sys.stderr, flush=True)
    result['runs'] = runs
    results = [r['result'] for r in runs]
    result['status'] = ('FAIL' if 'FAIL' in results else
                        'EXCLUDED' if 'EXCLUDED' in results else 'PASS')
    return result


def inside(work, binaries, campaign):
    box = Sandbox(pathlib.Path(work), pathlib.Path(binaries))
    try:
        result = {'dry-run': dry_run}[campaign](box)
    except Exception as error:  # every outcome is a record, never a traceback only
        result = {'campaign': campaign, 'status': 'EXCLUDED', 'reason': f'harness: {error}'}
    for name in ('provenance-certification.json', 'host.json'):
        result[name.removesuffix('.json')] = json.loads((box.work / name).read_text())
    write_json(box.work / 'results.json', result)
    for process in box.processes.values():
        if process.poll() is None:
            process.terminate()
    return 0 if result['status'] == 'PASS' else 1


def main():
    if len(sys.argv) > 1 and sys.argv[1] == '--inside':
        return inside(*sys.argv[2:5])
    parser = argparse.ArgumentParser()
    parser.add_argument('campaign', choices=['dry-run'])
    parser.add_argument('--keep-temp', action='store_true')
    return outside(parser.parse_args())


if __name__ == '__main__':
    sys.exit(main())
