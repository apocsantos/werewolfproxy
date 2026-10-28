#!/usr/bin/env python3
"""Stage32 robustness and chaos harness (docs/STAGE32_ROBUSTNESS_SPEC.md).

    python3 -B tests/integration/chaos.py dry-run [--keep-temp]
    python3 -B tests/integration/chaos.py network [--only S1,S2] [--from S3:P0]
        [--cells S2:P0,S5:P0] [--keep-temp]
    python3 -B tests/integration/chaos.py calibrate [--keep-temp]

Builds the featureless release certification build (and, for `network`, the
separate `lab-faults` build used only by S5's automatic-sync item), then runs
each matrix cell in its own unprivileged sandbox (`unshare -rmnp`: user,
mount, network and PID namespaces). Inside, a private root (read-only system
binds, chroot) lets the Den ancestor check run unchanged, one network
namespace per node is joined through a router namespace, and disposable
daemons are driven over their control sockets. Nothing in a sandbox can see
or reach any process or network outside it; when its first process ends,
everything in it ends.

Results go to target/stage32-chaos/results/<stamp>-<campaign>/: both builds'
provenance, the host facts, one JSON file per cell, logs of every cell that
did not pass, and summary.json. The network campaign stops at the first FAIL
(spec section 7).
"""
import argparse
import base64
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
LAB_TARGET = ROOT / 'target/stage32-chaos-lab'
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
ALL = list(PROFILES)
MATRIX = [('S1', ALL), ('S2', ALL), ('S3', ALL),
          ('S4', ['P0', 'P2', 'P3', 'P5', 'P7', 'P8']),
          ('S5', ['P0', 'P2', 'P3', 'P5', 'P8']),
          ('S6', ['P0', 'P2', 'P3']), ('S7', ['P0', 'P2', 'P3'])]
LEAK = {'fds': 4, 'threads': 2, 'rss_kib': 32 * 1024}
MIB = 1 << 20
OPEN_BOUND_S = 20
FENCE_BOUND_S = 10
# S1-S3 payloads (spec section 2): direct (S1, S2) and routed (S3). P9's are
# set by its rate. The others here are calibrated against raw TCP on the
# same path (spec, "P5 calibration" and "S3 calibration"): direct P5 in
# calibrate run 20261027T203757Z; routed P2, P3 and P5 on the relayed path
# in calibrate run 20261028T145605Z. None means not calibrated: P3's routed
# path moved under 1 MiB in 60 s, which the rule cannot express.
PAYLOAD = {'direct': 32 * MIB, 'routed': 16 * MIB}
PAYLOAD_BY_PROFILE = {'P9': {'direct': 4 * MIB, 'routed': 2 * MIB},
                      'P5': {'direct': 2 * MIB, 'routed': 2 * MIB},
                      'P2': {'routed': 1 * MIB},
                      'P3': {'routed': None}}
# The paths `calibrate` measures for each profile.
CALIBRATION_PATHS = {'P2': ('routed',), 'P3': ('routed',), 'P5': ('routed',)}
LAB_ENV = {'WEREWOLF_LAB_AUTO_SYNC_FIRST_MS': '300', 'WEREWOLF_LAB_AUTO_SYNC_INTERVAL_MS': '1500'}


def sh(*argv, check=True, capture=True, env=None):
    out = subprocess.run([str(a) for a in argv], capture_output=capture, text=True, env=env)
    if check and out.returncode != 0:
        raise RuntimeError(f'{" ".join(map(str, argv))}: {out.stderr.strip()}')
    return out


def log(message):
    print(message, file=sys.stderr, flush=True)


# --- outside the sandbox ----------------------------------------------------

def build(target, features):
    env = {k: v for k, v in os.environ.items()
           if not k.startswith('WEREWOLF_') and k not in ('BASH_ENV', 'ENV')}
    extra = ['--features', ','.join(features)] if features else []
    sh('cargo', 'build', '--release', '--locked', '--offline', '--workspace', '--bins',
       *extra, '--target-dir', target, env=env)
    binaries = target / 'release'
    tool = lambda *a: subprocess.check_output(a, env=env).decode().strip()
    return binaries, {
        'build': 'lab' if features else 'certification', 'features': features,
        'target_dir': str(target),
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


def cells(campaign, only, start, explicit=None):
    if campaign == 'dry-run':
        return [('S1', 'P0'), ('S1', 'P2')]
    if campaign == 'calibrate':
        return [('CAL', p) for p in CALIBRATION_PATHS]
    if explicit:
        return [tuple(c.split(':')) for c in explicit.split(',')]
    chosen = [(s, p) for s, profiles in MATRIX for p in profiles
              if not only or s in only]
    if start:
        surface, profile = start.split(':')
        chosen = chosen[chosen.index((surface, profile)):]
    return chosen


def outside(args):
    cert, cert_provenance = build(CERT_TARGET, [])
    lab, lab_provenance = (build(LAB_TARGET, ['werewolfd/lab-faults'])
                           if args.campaign == 'network' else (None, None))
    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())
    results = CERT_TARGET / 'results' / f'{stamp}-{args.campaign}'
    (results / 'cells').mkdir(parents=True)
    write_json(results / 'provenance-certification.json', cert_provenance)
    if lab_provenance:
        write_json(results / 'provenance-lab.json', lab_provenance)
    write_json(results / 'host.json', host_facts())
    only = set(args.only.split(',')) if args.only else None
    p0 = {}
    summary = []
    status = 0
    for surface, profile in cells(args.campaign, only, args.start, args.cells):
        work = pathlib.Path(tempfile.mkdtemp(prefix='wwp-chaos-'))
        os.chmod(work, 0o755)
        name = f'{surface}-{profile}'
        log(f'--- {name}')
        inner = subprocess.run(
            ['unshare', '-rmnp', '--fork', '--kill-child', '--',
             sys.executable, '-B', __file__, '--inside', str(work), str(cert),
             str(lab or ''), surface, profile, json.dumps(p0)])
        cell_file = work / 'cell.json'
        if cell_file.exists():
            cell = json.loads(cell_file.read_text())
        else:
            cell = {'surface': surface, 'profile': profile, 'status': 'EXCLUDED',
                    'reason': f'sandbox produced no result (exit {inner.returncode})'}
        write_json(results / 'cells' / f'{name}.json', cell)
        if cell['status'] != 'PASS' or args.keep_temp:
            kept = results / 'cells' / f'{name}-work'
            shutil.copytree(work, kept, symlinks=True,
                            ignore=shutil.ignore_patterns('root'))
        shutil.rmtree(work, ignore_errors=True)
        if profile == 'P0' and cell.get('p0_time_s'):
            p0[surface] = cell['p0_time_s']
        summary.append({'cell': name, 'status': cell['status'],
                        'reason': cell.get('reason'),
                        'runs': [r.get('result') for r in cell.get('runs', [])]})
        write_json(results / 'summary.json', {'campaign': args.campaign, 'p0_s': p0,
                                              'cells': summary})
        log(f'{cell["status"]:8} {name} {cell.get("reason") or ""}')
        if cell['status'] == 'FAIL':
            status = 1
            log(f'FAIL stops the campaign (spec section 7): {name}')
            break
        if cell['status'] == 'EXCLUDED':
            status = 1
    statuses = [c['status'] for c in summary]
    overall = ('FAIL' if 'FAIL' in statuses else 'EXCLUDED' if 'EXCLUDED' in statuses
               else 'PASS')
    final = {'campaign': args.campaign, 'status': overall, 'results': str(results),
             'p0_s': p0, 'cells': summary}
    write_json(results / 'summary.json', final)
    print(json.dumps(final, indent=1))
    return status


# --- inside the sandbox -----------------------------------------------------

class Sandbox:
    def __init__(self, work, cert, lab):
        self.work = work
        self.cert = cert
        self.lab = lab
        self.root = work / 'root'
        self.daemons = {}
        self.handles = []

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
        for name in ('proc', 'dev', 'bins', 'labbins', 'tmp'):
            (self.root / name).mkdir()
        sh('mount', '-t', 'proc', 'proc', self.root / 'proc')
        sh('mount', '--rbind', '/dev', self.root / 'dev')
        for source, target in ((self.cert, 'bins'), (self.lab, 'labbins')):
            if source:
                sh('mount', '--bind', source, self.root / target)
                sh('mount', '-o', 'remount,bind,ro', self.root / target)
        (self.root / 'work').mkdir(mode=0o700)
        (self.work / 'home').mkdir(mode=0o700)
        owners = {p: os.stat(self.root / p).st_uid for p in ('.', 'work')}
        read_only = {}
        for path in ('/usr/.wwp-ro-probe', '/etc/.wwp-ro-probe', '/bins/.wwp-ro-probe'):
            write = subprocess.run(['/usr/sbin/chroot', self.root, '/bin/touch', path],
                                   capture_output=True, text=True)
            read_only[path] = write.returncode != 0 and 'Read-only' in write.stderr
        facts = {'root_owner': owners['.'], 'work_owner': owners['work'],
                 'read_only': read_only, 'system_binds_read_only': all(read_only.values())}
        require(facts['root_owner'] == 0 and facts['work_owner'] == 0,
                f'private root not namespace-owned: {facts}')
        require(facts['system_binds_read_only'], f'system binds are writable: {facts}')
        return facts

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

    # Impairment: netem on both ends of a node's link, all traffic or UDP only.

    def ends(self, node, seed):
        return ((node, f'v{node}', seed), ('r', f'r{node}', seed + 1))

    def clear(self, node):
        for ns, dev, _ in self.ends(node, 0):
            self.in_net(ns, '/usr/sbin/tc', 'qdisc', 'del', 'dev', dev, 'root', check=False)

    def impair(self, node, profile, seed, udp_only=False):
        self.clear(node)
        spec = PROFILES[profile]
        commands = []
        if spec is None:
            return commands
        for ns, dev, s in self.ends(node, seed):
            tc = lambda *a: self.in_net(ns, '/usr/sbin/tc', *a)
            if udp_only:
                steps = [['qdisc', 'add', 'dev', dev, 'root', 'handle', '1:', 'prio', 'bands', '3',
                          'priomap', *(['1'] * 16)],
                         ['qdisc', 'add', 'dev', dev, 'parent', '1:3', 'handle', '30:', 'netem',
                          *spec.split(), 'seed', str(s)],
                         ['filter', 'add', 'dev', dev, 'parent', '1:', 'protocol', 'ip', 'prio',
                          '1', 'u32', 'match', 'ip', 'protocol', '17', '0xff', 'flowid', '1:3']]
            else:
                steps = [['qdisc', 'add', 'dev', dev, 'root', 'netem', *spec.split(),
                          'seed', str(s)]]
            for step in steps:
                tc(*step)
            shown = tc('qdisc', 'show', 'dev', dev).stdout.strip()
            require('netem' in shown, f'netem not applied on {ns}/{dev}: {shown}')
            commands.append({'ns': ns, 'tc': steps, 'shown': shown})
        return commands

    def netem_packets(self, node):
        """Packets that went through netem on the node's link, both ends."""
        total = 0
        for ns, dev, _ in self.ends(node, 0):
            text = self.in_net(ns, '/usr/sbin/tc', '-s', 'qdisc', 'show', 'dev', dev).stdout
            lines = text.splitlines()
            for i, line in enumerate(lines):
                if 'netem' in line and i + 1 < len(lines) and 'Sent' in lines[i + 1]:
                    total += int(lines[i + 1].split()[3])
        return total

    # Daemons. `name` is the Den (and socket); `node` the network namespace.

    def start(self, name, node, lab=False):
        den = self.root / 'work' / name
        den.mkdir(mode=0o700)
        write_json(den / 'silver.json', {'version': 1, 'mode': 'open'})
        address = f'{NODES[node]}.2:{PORT}'
        handle = (self.work / f'{name}.log').open('ab')
        self.handles.append(handle)
        env = {'PATH': '/usr/bin:/bin', **(LAB_ENV if lab else {})}
        process = subprocess.Popen(
            ['nsenter', f'--net=/run/netns/{node}', '/usr/sbin/chroot', self.root,
             '/labbins/werewolfd' if lab else '/bins/werewolfd',
             '--home', f'/work/{name}', '--socket', f'/work/{name}.sock',
             '--listen', address, '--quic-listen', address],
            stdout=handle, stderr=subprocess.STDOUT, env=env)
        self.daemons[name] = process
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            try:
                self.ctl(name, 'status')
                break
            except (OSError, RuntimeError):
                require(process.poll() is None, f'{name} exited {process.returncode}')
                time.sleep(.1)
        else:
            raise RuntimeError(f'{name} control not ready')
        self.ctl(name, 'pelt.init')
        self.ctl(name, 'state.manifest.migrate')
        return self.ctl(name, 'pelt.fingerprint')

    def stop(self, name):
        process = self.daemons.pop(name)
        process.terminate()
        process.wait(timeout=15)

    def ctl(self, name, cmd, args=None, timeout=10):
        return control_request(self.root / 'work' / f'{name}.sock', cmd, args, timeout)

    def try_ctl(self, name, cmd, args=None, timeout=10):
        try:
            return self.ctl(name, cmd, args, timeout)
        except (OSError, RuntimeError) as error:
            return {'error': str(error)}

    def pack_add(self, name, peer_name, ident, node):
        self.ctl(name, 'pack.add', {'name': peer_name, 'fingerprint': ident['fingerprint'],
                                    'public_key_b64': ident['public_key_b64'],
                                    'address': f'quic://{NODES[node]}.2:{PORT}'})

    def sample(self, name):
        process = self.daemons[name]
        pid = process.pid
        status = pathlib.Path(f'/proc/{pid}/status').read_text()
        field = lambda key: int(next(l.split()[1] for l in status.splitlines()
                                     if l.startswith(key + ':')))
        return {'fds': len(os.listdir(f'/proc/{pid}/fd')), 'threads': field('Threads'),
                'rss_kib': field('VmRSS'), 'alive': process.poll() is None}

    def targets(self, node, script='targets.py', *extra):
        directory = self.work / f'{pathlib.Path(script).stem}-{node}'
        directory.mkdir()
        handle = (self.work / f'{directory.name}.log').open('ab')
        self.handles.append(handle)
        subprocess.Popen(['nsenter', f'--net=/run/netns/{node}', sys.executable, '-I', '-B',
                          HERE / script, directory, *extra], stdout=handle,
                         stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 15
        while not (directory / 'ready.json').exists():
            require(time.monotonic() < deadline, 'targets not ready')
            time.sleep(.1)
        return json.loads((directory / 'ready.json').read_text())

    def echo(self, node, host='127.0.0.1'):
        """The echo target for data-carrying surfaces: no idle timeout."""
        return self.targets(node, 'chaos_echo.py', host)['echo']

    def client_argv(self, node, address, size, label, limit):
        return ['nsenter', f'--net=/run/netns/{node}', sys.executable, '-I', '-B',
                str(HERE / 'chaos_client.py'), 'duplex', address, str(size), label, str(limit)]

    def client(self, node, address, size, label, limit):
        out = sh(*self.client_argv(node, address, size, label, limit), check=False)
        return parse_client(out.returncode, out.stdout, out.stderr)

    def client_background(self, node, address, size, label, limit):
        return subprocess.Popen(self.client_argv(node, address, size, label, limit),
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    def fang(self, name, peer, local, remote, transport):
        opened = self.ctl(name, 'fang.open', {'peer': peer, 'local': local, 'remote': remote,
                                               'transport': transport}, timeout=30)
        return {k: opened.get(k) for k in ('transport', 'status', 'via') if k in opened}

    def close_fangs(self, name):
        for fang in self.ctl(name, 'fang.list'):
            self.ctl(name, 'fang.close', {'fang_id': fang['id']})

    def established(self, node, port):
        out = self.in_net(node, 'ss', '-tnH', 'state', 'established',
                          '(', 'dport', '=', f':{port}', ')')
        return len([l for l in out.stdout.splitlines() if l.strip()])

    def gov(self, *args):
        out = sh(self.cert / 'werewolfctl', '--json', 'governance', *args)
        return json.loads(out.stdout)['result']


def parse_client(code, stdout, stderr):
    lines = stdout.strip().splitlines()
    if code != 0 or not lines:
        return {'error': f'client exit {code}: {stderr.strip()[-300:]}', 'harness': True}
    return json.loads(lines[-1])


class Governance:
    """Ten throwaway keys in the cell's work directory; statements by file."""

    def __init__(self, box):
        self.box = box
        self.dir = box.work / 'gov'
        self.dir.mkdir(mode=0o700)
        self.keys = [box.gov('keygen', self.dir / f'k{i}.key')['public_key_b64']
                     for i in range(10)]
        self.count = 0

    def statement(self, draft_args):
        self.count += 1
        name = f's{self.count}'
        draft = self.dir / f'{name}.draft'
        self.box.gov('draft', *draft_args, '--out', draft)
        parts = []
        for i in range(7):
            part = self.dir / f'{name}.{i}.part'
            self.box.gov('sign', draft, self.dir / f'k{i}.key', '--index', str(i), '--out', part)
            parts.append(part)
        out = self.dir / f'{name}.wwgov'
        self.box.gov('combine', draft, *parts, '--out', out)
        return out

    def genesis(self):
        path = self.statement(['genesis', *[x for k in self.keys for x in ('--key', k)]])
        return path, self.box.gov('inspect', path)['statement']['hash']

    def dummy(self):
        return self.box.gov('keygen', self.dir / f'dummy{self.count}-{time.monotonic_ns()}.key'
                            )['public_key_b64']

    def apply(self, name, path, expect=None):
        args = {'statement': base64.b64encode(path.read_bytes()).decode()}
        if expect:
            args['expect_hash'] = expect
        return self.box.ctl(name, 'governance.apply', args)


# --- judging ----------------------------------------------------------------

def judge_transfer(transfer, profile, reasons, label=''):
    if transfer.get('harness'):
        return 'EXCLUDED'
    if transfer.get('complete') and not transfer.get('match'):
        reasons.append(f'{label}integrity: SHA-256 mismatch')
    if transfer.get('error') == 'outer bound exceeded':
        reasons.append(f'{label}liveness: outer bound exceeded')
    if profile == 'P0' and not transfer.get('complete'):
        reasons.append(f'{label}baseline transfer did not complete: {transfer.get("error")}')
    return None


def judge_leaks(box, baselines, reasons):
    after = {}
    for name, baseline in baselines.items():
        sample = box.sample(name)
        after[name] = sample
        if not sample['alive']:
            reasons.append(f'{name} exited')
            continue
        over = {k: sample[k] - baseline[k] for k in LEAK if sample[k] - baseline[k] > LEAK[k]}
        if over:
            reasons.append(f'{name} over leak margin: {over}')
    return after


def bound_for(profile, surface, p0):
    if profile == 'P9':
        return 900
    if profile == 'P0' or surface not in p0:
        return 120
    return max(120, 10 * p0[surface])


def payload(profile, kind):
    size = PAYLOAD_BY_PROFILE.get(profile, {}).get(kind, PAYLOAD[kind])
    require(size is not None, f'{profile} {kind} payload is not calibrated')
    return size


def finish(run, excluded, reasons):
    if excluded:
        run['result'], run['reason'] = 'EXCLUDED', excluded
    else:
        run['result'] = 'FAIL' if reasons else 'PASS'
        run['reason'] = '; '.join(reasons) or None
    return run


def pair(box, nodes=('a', 'b')):
    ids = {n: box.start(n, n) for n in nodes}
    for node in nodes:
        for peer in nodes:
            if peer != node:
                box.pack_add(node, f'wolf-{peer}', ids[peer], peer)
    return ids


# --- surfaces ---------------------------------------------------------------

def transfers(box, surface, profile, p0):
    """S1 (encrypted TCP), S2 (QUIC), S3 (routed through Y, QUIC and TCP hops)."""
    routed = surface == 'S3'
    nodes = ('a', 'b', 'y') if routed else ('a', 'b')
    box.build_network(nodes)
    echo = box.echo('b')
    if routed:
        ids = {n: box.start(n, n) for n in nodes}
        box.pack_add('a', 'wolf-b', ids['b'], 'b')
        box.pack_add('a', 'wolf-y', ids['y'], 'y')
        box.pack_add('b', 'wolf-a', ids['a'], 'a')
        box.pack_add('y', 'wolf-a', ids['a'], 'a')
        box.ctl('y', 'target.allow', {'peer': 'wolf-a', 'target': f'{NODES["b"]}.2:{PORT}'})
        legs = [('quic hop', 'quic'), ('tcp hop', 'tcp')]
        size = payload(profile, 'routed')
        links = ('a', 'y')
    else:
        pair(box)
        legs = [('encrypted tcp', 'tcp')] if surface == 'S1' else [('quic', 'quic')]
        size = payload(profile, 'direct')
        links = ('a',)
    box.ctl('b', 'target.allow', {'peer': 'wolf-a', 'target': f'127.0.0.1:{echo}'})
    time.sleep(2)
    baselines = {n: box.sample(n) for n in nodes}
    runs, times = [], []
    k = int(profile[1:])
    for n in (1, 2, 3):
        seed = 1000 * k + 10 * n
        run = {'run': n, 'seed': seed, 'build': 'certification', 'impaired': list(links)}
        run['netem'] = {link: box.impair(link, profile, seed + 100 * i)
                        for i, link in enumerate(links)}
        bound = bound_for(profile, surface, p0)
        run['outer_bound_s'] = bound
        reasons, excluded, run['legs'] = [], None, []
        for label, mode in legs:
            if routed:
                box.ctl('a', 'route.set', {'peer': 'wolf-b', 'via': 'wolf-y', 'hop': mode})
            fang = box.fang('a', 'wolf-b', '127.0.0.1:17001', f'127.0.0.1:{echo}',
                            'quic' if routed else mode)
            transfer = box.client('a', '127.0.0.1:17001', size, f'{surface}-{profile}-{n}-{mode}',
                                  bound)
            box.close_fangs('a')
            run['legs'].append({'leg': label, 'fang': fang, 'transfer': transfer})
            excluded = excluded or (transfer['error'] if judge_transfer(
                transfer, profile, reasons, f'{label}: ') else None)
            if profile == 'P0' and transfer.get('complete'):
                times.append(transfer['elapsed_s'])
        for link in links:
            box.clear(link)
        time.sleep(SETTLE_S)
        run['after_settle'] = judge_leaks(box, baselines, reasons)
        runs.append(finish(run, excluded, reasons))
        log(f'  {run["result"]:8} run {n}: ' + ', '.join(
            f'{leg["leg"]} {leg["transfer"].get("elapsed_s")} s '
            f'match={leg["transfer"].get("match")} error={leg["transfer"].get("error")}'
            for leg in run['legs']))
    return {'baselines': baselines, 'runs': runs,
            'p0_time_s': max(times) if profile == 'P0' and times else None}


def establishment(box, profile, p0):
    """S4: 50 sequential sessions of a 1 KiB echo, per transport."""
    box.build_network(('a', 'b'))
    echo = box.echo('b')
    pair(box)
    box.ctl('b', 'target.allow', {'peer': 'wolf-a', 'target': f'127.0.0.1:{echo}'})
    ports = {'quic': 17001, 'tcp': 17002}
    for mode, port in ports.items():
        box.fang('a', 'wolf-b', f'127.0.0.1:{port}', f'127.0.0.1:{echo}', mode)
    time.sleep(2)
    baselines = {n: box.sample(n) for n in ('a', 'b')}
    runs = []
    k = int(profile[1:])
    for n in (1, 2, 3):
        seed = 1000 * k + 10 * n
        run = {'run': n, 'seed': seed, 'build': 'certification', 'impaired': ['a'],
               'netem': box.impair('a', profile, seed)}
        reasons, excluded, run['transports'] = [], None, {}
        for mode, port in ports.items():
            outcomes = []
            for i in range(50):
                t = box.client('a', f'127.0.0.1:{port}', 1024, f'S4-{profile}-{n}-{mode}-{i}',
                               OPEN_BOUND_S)
                if t.get('harness'):
                    excluded = t['error']
                    break
                if t.get('complete') and not t.get('match'):
                    reasons.append(f'{mode} session {i}: integrity: SHA-256 mismatch')
                if t.get('error') == 'outer bound exceeded':
                    reasons.append(f'{mode} session {i}: did not end within {OPEN_BOUND_S} s')
                outcomes.append(t)
            ok = [t for t in outcomes if t.get('complete')]
            errors = {}
            for t in outcomes:
                if not t.get('complete'):
                    errors[t.get('error')] = errors.get(t.get('error'), 0) + 1
            times = sorted(t['elapsed_s'] for t in outcomes)
            run['transports'][mode] = {
                'sessions': len(outcomes), 'complete': len(ok), 'errors': errors,
                'elapsed_s': {'min': times[0], 'median': times[len(times) // 2],
                              'max': times[-1]} if times else None}
            if profile == 'P0' and len(ok) != len(outcomes):
                reasons.append(f'{mode}: baseline sessions failed: {errors}')
        box.clear('a')
        time.sleep(SETTLE_S)
        run['after_settle'] = judge_leaks(box, baselines, reasons)
        runs.append(finish(run, excluded, reasons))
        log(f'  {run["result"]:8} run {n}: ' + ', '.join(
            f'{m} {v["complete"]}/{v["sessions"]} max {v["elapsed_s"]["max"] if v["elapsed_s"] else None} s'
            for m, v in run['transports'].items()))
    return {'baselines': baselines, 'runs': runs}


def control_plane(box, profile, p0):
    """S5: discovery, a 4-statement manual sync (certification build), and
    a Stage31 automatic sync of a revocation (lab build)."""
    box.build_network(('a', 'b'))
    echo = box.targets('b')['echo']
    gov = Governance(box)
    runs = []
    k = int(profile[1:])
    for n in (1, 2, 3):
        seed = 1000 * k + 10 * n
        run = {'run': n, 'seed': seed, 'impaired': ['a']}
        reasons, excluded = [], None
        # Certification build: discovery and the manual sync.
        a, b = f'a{n}', f'b{n}'
        ids = {a: box.start(a, 'a'), b: box.start(b, 'b')}
        box.pack_add(a, 'wolf-b', ids[b], 'b')
        box.pack_add(b, 'wolf-a', ids[a], 'a')
        box.ctl(b, 'discover.advertise', {'addresses': f'quic://{NODES["b"]}.2:{PORT}'})
        box.ctl(b, 'target.allow', {'peer': 'wolf-a', 'target': f'127.0.0.1:{echo}'})
        box.ctl(b, 'service.publish', {'name': 'echo', 'target': f'127.0.0.1:{echo}',
                                       'kind': 'tcp'})
        genesis, ghash = gov.genesis()
        chain = []
        previous = genesis
        for body in (['admit', '--member', gov.dummy()], ['admit', '--member', gov.dummy()],
                     ['revoke', '--member', gov.dummy()], ['revoke', '--member', gov.dummy()]):
            previous = gov.statement([body[0], '--after', previous, *body[1:]])
            chain.append(previous)
        gov.apply(a, genesis, ghash)
        gov.apply(b, genesis, ghash)
        for statement in chain:
            gov.apply(b, statement)
        time.sleep(2)
        baselines = {a: box.sample(a), b: box.sample(b)}
        run['netem'] = box.impair('a', profile, seed)
        cert = {'build': 'certification'}
        refresh = box.try_ctl(a, 'discover.refresh', {'transport': 'quic'}, timeout=30)
        cert['discover_refresh'] = refresh
        services = box.try_ctl(a, 'discover.services', {'peer': 'wolf-b', 'transport': 'quic'},
                               timeout=30)
        cert['discover_services'] = services
        for label, value in (('discover refresh', refresh), ('discover services', services)):
            if 'error' in value and not isinstance(value, list):
                reasons.append(f'{label}: command failed: {value["error"]}')
        attempts, applied = [], []
        for attempt in range(3):
            synced = box.try_ctl(a, 'governance.sync', {'peer': '', 'transport': 'quic'},
                                 timeout=150)
            attempts.append(synced)
            if 'error' in synced:
                reasons.append(f'governance sync failed: {synced["error"]}')
                break
            applied += [r['sequence'] for r in synced.get('applied', [])]
            if box.ctl(a, 'governance.status')['sequence'] == 4:
                break
        cert['sync_attempts'] = attempts
        cert['applied'] = applied
        head = box.ctl(a, 'governance.status')['sequence']
        cert['head'] = head
        if applied != list(range(1, len(applied) + 1)):
            reasons.append(f'sync applied {applied}, not an in-order prefix of 1-4 each once')
        if head != len(applied):
            reasons.append(f'head {head} after applying {applied}')
        for seq in applied:
            archived = box.root / 'work' / a / f'governance_{seq:08d}.wwgov'
            if archived.read_bytes() != chain[seq - 1].read_bytes():
                reasons.append(f'archive {seq} differs from the file')
        if profile == 'P0' and head != 4:
            reasons.append(f'baseline sync did not reach 4: {applied}')
        box.clear('a')
        time.sleep(SETTLE_S)
        cert['after_settle'] = judge_leaks(box, baselines, reasons)
        box.stop(a)
        box.stop(b)
        run['certification'] = cert
        # Lab build: Stage31 automatic sync of a revocation.
        la, lb = f'la{n}', f'lb{n}'
        lab = {'build': 'lab'}
        lids = {la: box.start(la, 'a', lab=True), lb: box.start(lb, 'b', lab=True)}
        box.pack_add(la, 'wolf-b', lids[lb], 'b')
        box.pack_add(lb, 'wolf-a', lids[la], 'a')
        lgenesis, lhash = gov.genesis()
        admit = gov.statement(['admit', '--after', lgenesis,
                               '--member', lids[la]['public_key_b64'],
                               '--member', lids[lb]['public_key_b64']])
        activate = gov.statement(['activate', '--after', admit])
        for name in (la, lb):
            gov.apply(name, lgenesis, lhash)
            gov.apply(name, admit)
            gov.apply(name, activate)
        time.sleep(2)
        lbaselines = {la: box.sample(la), lb: box.sample(lb)}
        box.impair('a', profile, seed)
        revoke = gov.statement(['revoke', '--after', activate, '--member', gov.dummy()])
        gov.apply(lb, revoke)
        started = time.monotonic()
        bound = 120
        marker = '[AUTO_SYNC_APPLIED] '
        def auto_applied():
            text = (box.work / f'{la}.log').read_text(errors='replace')
            return [json.loads(l.split(marker, 1)[1]) for l in text.splitlines() if marker in l]
        while time.monotonic() - started < bound:
            if any(r.get('sequence') == 3 for r in auto_applied()):
                break
            time.sleep(.5)
        lab['seconds_to_apply'] = round(time.monotonic() - started, 1)
        status = box.ctl(la, 'governance.status')
        lab['last_run'] = status['auto_sync']['last_run']
        box.clear('a')
        time.sleep(SETTLE_S)
        sequences = [r.get('sequence') for r in auto_applied()]
        lab['automatic_applies'] = sequences
        if sequences.count(3) > 1:
            reasons.append(f'automatic sync applied sequence 3 {sequences.count(3)} times')
        if 3 not in sequences:
            peers = (lab['last_run'] or {}).get('peers', [])
            reached = any(p.get('status') == 'ok' for p in peers)
            if profile == 'P0' or reached:
                reasons.append(f'automatic sync did not apply the revocation within {bound} s')
            else:
                lab['not_converged'] = f'peer not reached within {bound} s'
        if box.ctl(la, 'governance.status')['sequence'] != (3 if 3 in sequences else 2):
            reasons.append('lab node head does not match its automatic applies')
        lab['after_settle'] = judge_leaks(box, lbaselines, reasons)
        box.stop(la)
        box.stop(lb)
        run['lab'] = lab
        runs.append(finish(run, excluded, reasons))
        log(f'  {run["result"]:8} run {n}: manual applied {applied}, '
            f'automatic {sequences} in {lab["seconds_to_apply"]} s')
    return {'runs': runs}


def fences(box, profile, p0):
    """S6: Silver, then a Pack revoke of A, on B during transfers; timed at B."""
    box.build_network(('a', 'b'))
    echo = box.echo('b')
    ids = pair(box)
    box.ctl('b', 'target.allow', {'peer': 'wolf-a', 'target': f'127.0.0.1:{echo}'})
    ports = {'tcp': 17001, 'quic': 17002}
    for mode, port in ports.items():
        box.fang('a', 'wolf-b', f'127.0.0.1:{port}', f'127.0.0.1:{echo}', mode)
    time.sleep(2)
    baselines = {n: box.sample(n) for n in ('a', 'b')}
    runs = []
    k = int(profile[1:])
    for n in (1, 2, 3):
        seed = 1000 * k + 10 * n
        run = {'run': n, 'seed': seed, 'build': 'certification', 'impaired': ['a'],
               'netem': box.impair('a', profile, seed), 'fences': []}
        reasons, excluded = [], None
        for fence, mode in (('silver', 'tcp'), ('revoke', 'quic')):
            port = ports[mode]
            record = {'fence': fence, 'transport': mode}
            background = box.client_background('a', f'127.0.0.1:{port}', 256 * MIB,
                                               f'S6-{profile}-{n}-{fence}', 300)
            time.sleep(1.5)
            record['b_before'] = {'active': box.ctl('b', 'status')['inbound_authority']['active'],
                                  'target_connections': box.established('b', echo)}
            if fence == 'silver':
                box.ctl('b', 'silver.trigger')
            else:
                box.ctl('b', 'pack.revoke', {'name': 'wolf-a'})
            t0 = time.monotonic()
            closed_at = None
            while time.monotonic() - t0 < FENCE_BOUND_S + 5:
                active = box.ctl('b', 'status')['inbound_authority']['active']
                if active == 0 and box.established('b', echo) == 0:
                    closed_at = time.monotonic() - t0
                    break
                time.sleep(.05)
            record['b_closed_after_s'] = None if closed_at is None else round(closed_at, 3)
            if record['b_before']['active'] == 0:
                excluded = excluded or f'{fence}: no session was active at B when fenced'
            elif closed_at is None or closed_at > FENCE_BOUND_S:
                reasons.append(f'{fence}: B still had the session after {FENCE_BOUND_S} s')
            refused = box.client('a', f'127.0.0.1:{port}', 1024, f'S6-{fence}-after',
                                 OPEN_BOUND_S)
            record['open_while_fenced'] = refused
            if refused.get('complete'):
                reasons.append(f'{fence}: a new session succeeded while fenced')
            if refused.get('error') == 'outer bound exceeded':
                reasons.append(f'{fence}: the refused open did not end within {OPEN_BOUND_S} s')
            try:
                out, err = background.communicate(timeout=60)
                record['a_transfer'] = parse_client(background.returncode, out, err)
            except subprocess.TimeoutExpired:
                background.kill()
                record['a_transfer'] = {'note': 'still running 60 s after the fence; killed'}
            transfer = record['a_transfer']
            if transfer.get('complete') and not transfer.get('match'):
                reasons.append(f'{fence}: integrity: SHA-256 mismatch')
            # Restore, and show the path works again (recorded).
            if fence == 'silver':
                box.ctl('b', 'silver.reset')
            else:
                box.pack_add('b', 'wolf-a', ids['a'], 'a')
                box.try_ctl('b', 'target.allow', {'peer': 'wolf-a',
                                                   'target': f'127.0.0.1:{echo}'})
            restored = None
            for _ in range(3):
                restored = box.client('a', f'127.0.0.1:{port}', 1024, f'S6-{fence}-restored',
                                      OPEN_BOUND_S)
                if restored.get('complete'):
                    break
            record['open_after_restore'] = restored
            if profile == 'P0' and not restored.get('complete'):
                reasons.append(f'{fence}: baseline path did not work after restore')
            run['fences'].append(record)
        box.clear('a')
        time.sleep(SETTLE_S)
        run['after_settle'] = judge_leaks(box, baselines, reasons)
        runs.append(finish(run, excluded, reasons))
        log(f'  {run["result"]:8} run {n}: ' + ', '.join(
            f'{f["fence"]} closed at B in {f["b_closed_after_s"]} s' for f in run['fences']))
    return {'baselines': baselines, 'runs': runs}


def selection(box, profile, p0):
    """S7: the secure selector with only QUIC (UDP) impaired."""
    box.build_network(('a', 'b'))
    http = box.targets('b')['http']
    pair(box)
    box.ctl('b', 'target.allow', {'peer': 'wolf-a', 'target': f'127.0.0.1:{http}'})
    ports = {'quic': 17101, 'tcp': 17102, 'tcp-plain': 17103}
    for mode, port in ports.items():
        box.fang('a', 'wolf-b', f'127.0.0.1:{port}', f'127.0.0.1:{http}', mode)
    url = lambda port: f'http://127.0.0.1:{port}/'
    env = {'PATH': '/usr/bin:/bin', 'HOME': str(box.work / 'home'),
           'WEREWOLF_TARGET_URL': url(ports['quic']), 'WEREWOLF_QUIC_URL': url(ports['quic']),
           'WEREWOLF_TCP_ENC_URL': url(ports['tcp']), 'WEREWOLF_TCP_URL': url(ports['tcp-plain'])}
    time.sleep(2)
    baselines = {n: box.sample(n) for n in ('a', 'b')}
    runs = []
    k = int(profile[1:])
    for n in (1, 2, 3):
        seed = 1000 * k + 10 * n
        run = {'run': n, 'seed': seed, 'build': 'certification', 'impaired': ['a (UDP only)'],
               'netem': box.impair('a', profile, seed, udp_only=True), 'selections': []}
        reasons, excluded = [], None
        for i in range(10):
            out = box.in_net('a', 'bash', ROOT / 'scripts/wolf-b.sh', 'auto', '--policy',
                             'secure', '--json', check=False, env=env)
            try:
                value = json.loads(out.stdout.strip().splitlines()[-1])
            except (ValueError, IndexError):
                excluded = excluded or f'selector output unreadable (exit {out.returncode})'
                continue
            run['selections'].append({'exit': out.returncode, 'transport': value.get('transport'),
                                      'security_downgrade': value.get('security_downgrade')})
            if value.get('transport') == 'tcp-plain' or value.get('security_downgrade'):
                reasons.append(f'selection {i}: downgrade to {value.get("transport")}')
        if profile != 'P0':
            run['udp_packets_through_netem'] = box.netem_packets('a')
            if run['udp_packets_through_netem'] == 0:
                excluded = excluded or 'no UDP packet went through netem'
        chosen = {}
        for s in run['selections']:
            chosen[s['transport']] = chosen.get(s['transport'], 0) + 1
        run['chosen'] = chosen
        box.clear('a')
        time.sleep(SETTLE_S)
        run['after_settle'] = judge_leaks(box, baselines, reasons)
        runs.append(finish(run, excluded, reasons))
        log(f'  {run["result"]:8} run {n}: {chosen}')
    return {'baselines': baselines, 'runs': runs}


SURFACES = {'S1': transfers, 'S2': transfers, 'S3': transfers, 'S4': establishment,
            'S5': control_plane, 'S6': fences, 'S7': selection}


def calibration(box, profile, p0):
    """The workload rule (spec, "P5 calibration", "S3 calibration"): raw
    kernel TCP, no Werewolf, with the cell's seeds, on the S1/S2 path (A to
    B, the A-R link impaired) or the S3 path (A through a plain TCP relay on
    Y to B, the A-R and Y-R links impaired: two TCP legs, as the TCP hop).
    The payload is the largest power-of-two MiB that all three seeds
    complete within half the 120 s bound: probe how much each seed echoes
    in that time, then confirm the candidate, halving until all complete."""
    half = 120 / 2
    box.build_network(('a', 'b', 'y'))
    b = f'{NODES["b"]}.2'
    echo = f'{b}:{box.echo("b", b)}'
    k = int(profile[1:])
    seeds = [1000 * k + 10 * n for n in (1, 2, 3)]
    runs = []
    for kind in CALIBRATION_PATHS[profile]:
        if kind == 'direct':
            address, links = echo, ('a',)
        else:
            y = f'{NODES["y"]}.2'
            relay = box.targets('y', 'chaos_relay.py', y, echo)['relay']
            address, links = f'{y}:{relay}', ('a', 'y')

        def batch(size, stage):
            out = []
            for seed in seeds:
                netem = {link: box.impair(link, profile, seed + 100 * i)
                         for i, link in enumerate(links)}
                t = box.client('a', address, size, f'CAL-{profile}-{kind}-{seed}-{size}', half)
                for link in links:
                    box.clear(link)
                out.append({'path': kind, 'stage': stage, 'seed': seed, 'netem': netem,
                            'bytes': size, **{k: t.get(k) for k in (
                                'elapsed_s', 'received', 'complete', 'match', 'error')}})
                log(f'  {kind} {stage} seed {seed}: {t.get("received")} B in '
                    f'{t.get("elapsed_s")} s complete={t.get("complete")}')
            return out

        probe = batch(PAYLOAD[kind], 'probe')
        least = min(r['received'] for r in probe)
        size = MIB
        while size * 2 <= least:
            size *= 2
        require(least >= MIB, f'{kind}: under 1 MiB echoed within {half:.0f} s')
        confirmations = []
        while True:
            confirm = batch(size, 'confirm')
            confirmations += confirm
            if all(r['complete'] and r['match'] for r in confirm) or size == MIB:
                break
            size //= 2
        chosen = size if all(r['complete'] and r['match'] for r in confirm) else None
        runs.append({'path': kind, 'result': 'PASS' if chosen else 'EXCLUDED',
                     'reason': None if chosen else f'{kind}: 1 MiB not confirmed',
                     'payload_bytes': chosen, 'probe': probe, 'confirm': confirmations})
        log(f'  {kind}: payload {chosen} B')
    return {'runs': runs}


SURFACES['CAL'] = calibration


def inside(work, cert, lab, surface, profile, p0_json):
    box = Sandbox(pathlib.Path(work), pathlib.Path(cert),
                  pathlib.Path(lab) if lab else None)
    cell = {'surface': surface, 'profile': profile, 'netem': PROFILES[profile]}
    try:
        cell['sandbox'] = box.build_root()
        function = SURFACES[surface]
        p0 = json.loads(p0_json)
        outcome = (function(box, surface, profile, p0) if function is transfers
                   else function(box, profile, p0))
        cell.update(outcome)
        results = [r['result'] for r in cell['runs']]
        cell['status'] = ('FAIL' if 'FAIL' in results else
                          'EXCLUDED' if 'EXCLUDED' in results else 'PASS')
        cell['reason'] = '; '.join(r['reason'] for r in cell['runs'] if r.get('reason')) or None
    except Exception as error:  # every outcome is a record
        cell['status'], cell['reason'] = 'EXCLUDED', f'harness: {type(error).__name__}: {error}'
    write_json(box.work / 'cell.json', cell)
    for name in list(box.daemons):
        box.daemons[name].terminate()
    return 0


def main():
    if len(sys.argv) > 1 and sys.argv[1] == '--inside':
        return inside(*sys.argv[2:8])
    parser = argparse.ArgumentParser()
    parser.add_argument('campaign', choices=['dry-run', 'network', 'calibrate'])
    parser.add_argument('--only', help='comma-separated surfaces, e.g. S1,S2')
    parser.add_argument('--from', dest='start', help='resume at a cell, e.g. S3:P0')
    parser.add_argument('--cells', help='exactly these cells, e.g. S2:P0,S5:P0 (smoke tests)')
    parser.add_argument('--keep-temp', action='store_true')
    return outside(parser.parse_args())


if __name__ == '__main__':
    sys.exit(main())
