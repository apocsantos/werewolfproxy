#!/usr/bin/env python3
"""Stage42 M1: Fang connection flood reproduction (disposable lab).

Reuses the acceptance lab (tests/integration/run.py): fresh Dens, dynamic
loopback ports, the checkout build, and only its own process groups are
stopped. No live wolf is touched. Each scenario runs in a fresh lab.

  fd:<kind>     B runs under `prlimit --nofile=LIMIT`; one Fang of <kind>
                (quic, tcp, tcp-plain) is flooded until the daemon runs out of
                descriptors. Records B's descriptor count, CPU while saturated
                and after release, whether the Fang still listens, and whether
                a new legitimate echo works after the flood is released.
                `cpu_seconds_flood` (Stage47 T1) is B's CPU time from the
                start of the flood to the end of the hold; `cpu_flooded_pct`
                is a 5 s window after it, kept for comparison with earlier
                stages but sensitive to when the work ends.
  quota:<kind>  no descriptor limit; FLOOD connections through one Fang, each
                sends 16 bytes and waits for its echo. Records how many get an
                echo (the receiver allows 64 sessions per peer), B's descriptor
                count, and whether a legitimate echo works during the flood,
                and `cpu_seconds_flood` from the flood's start to its release.

Prints one JSON line per scenario; writes all to --out.
"""
import argparse
import json
import os
import pathlib
import select
import socket
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/integration'))
sys.dont_write_bytecode = True
from run import Lab, require, write_json  # noqa: E402

CLK_TCK = os.sysconf('SC_CLK_TCK')
# The lab lives on /tmp (a small tmpfs on DarkSide). A Fang spinning on EMFILE
# logs every retry, so every wait checks the logs and aborts above this size.
LOG_CAP = 50 * 1024 * 1024


class LogCap(RuntimeError):
    pass


class FloodLab(Lab):
    nofile = None

    def spawn(self, argv, label, env=None):
        if self.nofile and label.startswith('b-'):
            # prlimit execs the daemon in place: same PID, same /proc/PID/exe.
            argv = ['/usr/bin/prlimit', f'--nofile={self.nofile}:{self.nofile}', '--', *argv]
        return super().spawn(argv, label, env)

    def setup(self, kind, bind='127.0.0.1', echo_port=None):
        self.prepare_binaries()
        self.reserve_wolf_ports()
        self.reserve('fang')
        target_dir = self.base / 'targets'
        target_dir.mkdir()
        (target_dir / 'baseline.txt').write_bytes(b'x')
        target, _ = self.spawn([sys.executable, ROOT / 'tests/integration/targets.py', target_dir],
                               'targets')
        ready = self.wait_for(lambda: json.loads((target_dir / 'ready.json').read_text()),
                              'targets', target)
        self.ports.update(target=ready['http'], echo=echo_port or ready['echo'])
        self.start_wolves()
        identities = {}
        for wolf in ('a', 'b'):
            self.control(wolf, 'pelt.init')
            pelt = json.loads((self.base / wolf / 'pelt.json').read_text())
            identities[wolf] = {k: pelt[k] for k in ('fingerprint', 'public_key_b64')}
        for i, wolf in enumerate(('a', 'b')):
            self.wait_for(lambda wolf=wolf: self.listener(wolf + '_tcp'), f'{wolf} TCP', self.wolves[i])
        self.stop_wolves()

        def peer(name, wolf):
            return dict(name=name, trust='Packmate', **identities[wolf],
                        address=f'quic://{self.address(wolf + "_quic")}')
        write_json(self.base / 'a' / 'pack.json', [peer('wolf-b', 'b')])
        write_json(self.base / 'b' / 'pack.json', [peer('wolf-a', 'a')])
        write_json(self.base / 'a' / 'target_policy.json', {
            'mode': 'deny-by-default',
            'peers': {identities['b']['fingerprint']: {'targets': [
                {'address': '127.0.0.1', 'port': self.ports['echo']}]}}})
        write_json(self.base / 'b' / 'target_policy.json', {'mode': 'deny-by-default', 'peers': {}})
        self.start_wolves()
        self.release('fang')
        self.control('b', 'fang.profile.add', dict(
            name='flood', peer='wolf-a', local=f"{bind}:{self.ports['fang']}",
            remote=self.address('echo'), transport=kind))
        self.control('b', 'fang.open_profile', {'name': 'flood'})
        self.wait_for(lambda: listening(self.ports['fang'], bind), 'flood Fang', self.wolves[1])
        require(echo_ok(self.ports['fang'], 5, bind), 'baseline echo through the Fang failed')

    @property
    def b_pid(self):
        return self.wolves[1].pid

    def guard(self):
        sizes = {p.name: p.stat().st_size for p in self.base.glob('*.log')}
        big = {k: v for k, v in sizes.items() if v > LOG_CAP}
        if big:
            # Record the spinning daemon's state, then stop the daemons at
            # once; cleanup() stops the rest.
            try:
                self.capped = {'fds': self.fds(), 'cpu_pct_1s': self.cpu_sample(1),
                               'log_bytes': big}
            except OSError:
                self.capped = {'log_bytes': big}
            self.stop_wolves()
            raise LogCap(f'log cap exceeded, daemons stopped: {big}')

    def wait(self, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.guard()
            time.sleep(min(0.5, max(0, deadline - time.monotonic())))
        self.guard()

    def log_bytes(self, label):
        return sum(p.stat().st_size for p in self.base.glob(f'*-{label}.log'))

    def fds(self):
        return len(os.listdir(f'/proc/{self.b_pid}/fd'))

    def cpu_sample(self, seconds):
        def ticks():
            fields = pathlib.Path(f'/proc/{self.b_pid}/stat').read_text().rsplit(')', 1)[1].split()
            return int(fields[11]) + int(fields[12])
        start = ticks()
        time.sleep(seconds)
        return round(100 * (ticks() - start) / CLK_TCK / seconds, 1)

    def cpu_ticks(self):
        fields = pathlib.Path(f'/proc/{self.b_pid}/stat').read_text().rsplit(')', 1)[1].split()
        return int(fields[11]) + int(fields[12])

    def cpu_percent(self, seconds=5):
        def ticks():
            fields = pathlib.Path(f'/proc/{self.b_pid}/stat').read_text().rsplit(')', 1)[1].split()
            return int(fields[11]) + int(fields[12])
        start = ticks()
        self.wait(seconds)
        return round(100 * (ticks() - start) / CLK_TCK / seconds, 1)

    def fang_listed(self):
        try:
            return any(f['local'] == self.address('fang') for f in self.control('b', 'fang.list'))
        except Exception as exc:  # the control socket may itself be starved
            return f'control failed: {exc}'


def echo_ok(port, timeout, host='127.0.0.1', source=None):
    try:
        with socket.create_connection((host, port), timeout=timeout,
                                      source_address=(source, 0) if source else None) as s:
            s.settimeout(timeout)
            payload = os.urandom(1024)
            s.sendall(payload)
            got = b''
            while len(got) < len(payload):
                chunk = s.recv(65536)
                if not chunk:
                    return False
                got += chunk
            return got == payload
    except OSError:
        return False


def listening(port, host='127.0.0.1'):
    """True if a new connection completes at all (kernel backlog included)."""
    try:
        with socket.create_connection((host, port), timeout=2):
            return True
    except OSError:
        return False


def flood(port, count, payload=b'0123456789abcdef', lab=None):
    socks, errors, timeouts = [], {}, 0
    for i in range(count):
        if lab is not None and i % 5 == 0:
            lab.guard()
        if timeouts >= 10:  # backlog full: further connects only wait
            errors['stopped_after_timeouts'] = count - i
            break
        try:
            s = socket.create_connection(('127.0.0.1', port), timeout=0.5)
            s.sendall(payload)
            s.setblocking(False)
            socks.append(s)
            timeouts = 0
        except OSError as exc:
            if isinstance(exc, TimeoutError):
                timeouts += 1
            errors[type(exc).__name__] = errors.get(type(exc).__name__, 0) + 1
    return socks, errors


def count_echoes(socks, size, wait):
    got = {s: b'' for s in socks}
    closed = set()
    deadline = time.monotonic() + wait
    while time.monotonic() < deadline:
        pending = [s for s in socks if len(got[s]) < size and s not in closed]
        if not pending:
            break
        ready, _, _ = select.select(pending[:1000], [], [], 0.2)
        for s in ready:
            try:
                chunk = s.recv(65536)
            except OSError:
                closed.add(s)
                continue
            if not chunk:
                closed.add(s)
            got[s] += chunk
    return sum(len(v) >= size for v in got.values()), len(closed)


def scenario_fd(lab, kind, limit, max_conns):
    lab.setup(kind)
    port = lab.ports['fang']
    result = {'scenario': f'fd:{kind}', 'nofile': limit, 'fds_idle': lab.fds()}
    log_before = lab.log_bytes('b-2')
    started = time.monotonic()
    ticks = lab.cpu_ticks()
    socks, errors = flood(port, max_conns, lab=lab)
    flood_seconds = round(time.monotonic() - started, 1)
    lab.wait(3)
    cpu = lab.cpu_percent()
    result.update(client_connections=len(socks), client_errors=errors,
                  flood_seconds=flood_seconds,
                  fds_flooded=lab.fds(), cpu_flooded_pct=cpu,
                  cpu_seconds_flood=round((lab.cpu_ticks() - ticks) / CLK_TCK, 2),
                  b_log_growth_bytes=lab.log_bytes('b-2') - log_before)
    result['listening_flooded'] = listening(port)
    for s in socks:
        s.close()
    lab.wait(5)
    result.update(fds_released=lab.fds(), cpu_released_pct=lab.cpu_percent(),
                  listening_released=listening(port), fang_listed=lab.fang_listed(),
                  legit_echo_after=echo_ok(port, 10))
    result['daemon_alive'] = lab.wolves[1].poll() is None
    return result


def scenario_quota(lab, kind, count):
    lab.setup(kind)
    port = lab.ports['fang']
    result = {'scenario': f'quota:{kind}', 'fds_idle': lab.fds()}
    ticks = lab.cpu_ticks()
    socks, errors = flood(port, count, lab=lab)
    echoed, closed = count_echoes(socks, 16, 20)
    result.update(client_connections=len(socks), client_errors=errors,
                  echoed=echoed, closed_by_fang=closed, fds_flooded=lab.fds(),
                  legit_echo_during=echo_ok(port, 10),
                  cpu_seconds_flood=round((lab.cpu_ticks() - ticks) / CLK_TCK, 2))
    for s in socks:
        s.close()
    lab.wait(5)
    result.update(fds_released=lab.fds(), legit_echo_after=echo_ok(port, 10))
    # Stage42 M3: a longer settle, to tell slow teardown from a leak.
    lab.wait(25)
    result.update(fds_released_30s=lab.fds(),
                  daemon_alive=lab.wolves[1].poll() is None)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('scenarios', nargs='+', help='e.g. fd:quic fd:tcp quota:quic')
    parser.add_argument('--nofile', type=int, default=256)
    parser.add_argument('--max-conns', type=int, default=600)
    parser.add_argument('--flood', type=int, default=500)
    parser.add_argument('--out', type=pathlib.Path, required=True)
    parser.add_argument('--keep-temp', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    results = []
    for name in args.scenarios:
        mode, kind = name.split(':')
        lab = FloodLab(args.keep_temp)
        try:
            if mode == 'fd':
                lab.nofile = args.nofile
                row = scenario_fd(lab, kind, args.nofile, args.max_conns)
            else:
                row = scenario_quota(lab, kind, args.flood)
        except Exception as exc:
            row = {'scenario': name, 'error': str(exc)}
            if getattr(lab, 'capped', None):
                row['capped'] = lab.capped
        finally:
            row['lab'] = str(lab.base)
            row['daemon_sha256'] = getattr(lab, 'binary_hash', None)
            try:
                lab.cleanup()
            except Exception as exc:
                row['cleanup_error'] = str(exc)
        print(json.dumps(row), flush=True)
        results.append(row)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    main()
