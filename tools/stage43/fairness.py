#!/usr/bin/env python3
"""Stage43 M1: fairness within a Fang, reproduction (disposable lab).

Reuses the Stage42 flood lab (tools/stage42/fang_flood.py), which reuses the
acceptance lab: fresh Dens, dynamic ports, the checkout build, only its own
process groups are stopped, and the 50 MB log guard applies. No live wolf is
touched. Each scenario runs in a fresh lab.

  idle:<kind>    scenario A (survey §4). One client opens connections one at
                 a time, each echoes 16 bytes and then stays open and idle,
                 until the Fang refuses one. After IDLE seconds (default 65,
                 past the proposed T = 60 s) a legitimate client connects.
                 Its echo target has no idle timeout (the lab's closes idle
                 connections after 10 s).
                 Records whether the idle connections still hold their slots
                 and whether the legitimate echo works.
  source:<kind>  scenario D. Runs in its own user and network namespace
                 (`unshare -rmn`, a private root and 512 MB /tmp), with 10.43.0.1-3 on lo. The Fang binds
                 10.43.0.1. Source 10.43.0.2 holds connections that send 16
                 bytes every second (an active flood); source 10.43.0.3 then
                 tries a legitimate echo, at once and after 30 s.

<kind> is quic, tcp or tcp-plain. Prints one JSON line per scenario; writes
all to --out.
"""

import argparse
import json
import os
import pathlib
import select
import socket
import socketserver
import subprocess
import sys
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/stage42'))
sys.dont_write_bytecode = True
from fang_flood import FloodLab, LogCap, echo_ok  # noqa: E402

FANG_HOST = '10.43.0.1'
FLOOD_SOURCE = '10.43.0.2'
LEGIT_SOURCE = '10.43.0.3'
PAYLOAD = b'0123456789abcdef'


def open_echoed(host, port, source=None, timeout=5):
    """One connection that has echoed PAYLOAD, or None if refused/reset."""
    try:
        s = socket.create_connection((host, port), timeout=timeout,
                                     source_address=(source, 0) if source else None)
        s.settimeout(timeout)
        s.sendall(PAYLOAD)
        got = b''
        while len(got) < len(PAYLOAD):
            chunk = s.recv(65536)
            if not chunk:
                s.close()
                return None
            got += chunk
        return s
    except OSError:
        return None


def still_open(socks):
    """How many of socks have neither EOF nor an error pending."""
    alive = 0
    for s in socks:
        s.setblocking(False)
        try:
            alive += s.recv(1, socket.MSG_PEEK) is None
        except BlockingIOError:
            alive += 1
        except OSError:
            pass
    return alive


def fill(lab, host, port, source=None, limit=80):
    """Open echoed connections one by one until the Fang refuses one."""
    socks, refused = [], 0
    for i in range(limit):
        if i % 5 == 0:
            lab.guard()
        s = open_echoed(host, port, source)
        if s is None:
            refused += 1
            if refused >= 3:
                break
            continue
        socks.append(s)
    return socks, refused


def pump(lab, socks, seconds):
    """Keep socks active: 16 bytes each per second, draining echoes."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        lab.guard()
        for s in socks:
            try:
                s.send(PAYLOAD)
            except OSError:
                pass
        end = time.monotonic() + 1
        while time.monotonic() < end:
            ready, _, _ = select.select(socks, [], [], 0.2)
            for s in ready:
                try:
                    s.recv(65536)
                except OSError:
                    pass


class PatientEcho(socketserver.BaseRequestHandler):
    """Echo with no idle timeout (the lab's echo target closes after 10 s)."""
    def handle(self):
        while data := self.request.recv(65536):
            self.request.sendall(data)


def patient_echo():
    server = socketserver.ThreadingTCPServer(('127.0.0.1', 0), PatientEcho)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def scenario_idle(lab, kind, idle):
    echo = patient_echo()
    lab.setup(kind, echo_port=echo.server_address[1])
    port = lab.ports['fang']
    result = {'scenario': f'idle:{kind}', 'idle_s': idle}
    socks, refused = fill(lab, '127.0.0.1', port)
    result.update(held=len(socks), refused_while_filling=refused,
                  legit_at_ceiling=echo_ok(port, 5))
    lab.wait(idle)
    result.update(held_after_idle=still_open(socks),
                  legit_after_idle=echo_ok(port, 5),
                  fds_flooded=lab.fds())
    for s in socks:
        s.close()
    lab.wait(5)
    result.update(legit_after_release=echo_ok(port, 10),
                  daemon_alive=lab.wolves[1].poll() is None)
    echo.shutdown()
    return result


def scenario_source(lab, kind, active):
    lab.setup(kind, bind=FANG_HOST)
    port = lab.ports['fang']
    result = {'scenario': f'source:{kind}', 'active_s': active,
              'legit_before': echo_ok(port, 5, FANG_HOST, LEGIT_SOURCE)}
    socks, refused = fill(lab, FANG_HOST, port, FLOOD_SOURCE)
    result.update(flood_held=len(socks), refused_while_filling=refused,
                  legit_during_0s=echo_ok(port, 5, FANG_HOST, LEGIT_SOURCE))
    pump(lab, socks, active)
    result.update(flood_still_open=still_open(socks),
                  legit_during_after_active=echo_ok(port, 5, FANG_HOST, LEGIT_SOURCE))
    for s in socks:
        s.close()
    lab.wait(5)
    result.update(legit_after_release=echo_ok(port, 10, FANG_HOST, LEGIT_SOURCE),
                  daemon_alive=lab.wolves[1].poll() is None)
    return result


def sh(*argv):
    subprocess.run([str(a) for a in argv], check=True)


def enter_netns():
    """Re-exec inside a fresh user, mount and network namespace.

    The daemon checks every ancestor of its home is root- or self-owned
    (werewolf-core local_fs). In a user namespace the host's `/` belongs to an
    unmapped owner, so, as the Stage32 chaos sandbox does, build a private root
    the namespace owns: read-only binds of the system directories, `/home`
    (the checkout and its build), the host's `/proc` and `/dev`, and a private
    512 MB `/tmp` (which also keeps the lab off the host `/tmp`).
    """
    if os.environ.get('WWP_STAGE43_NETNS') != '1':
        env = dict(os.environ, WWP_STAGE43_NETNS='1')
        argv = ['unshare', '-rmn', '--fork', '--kill-child', '--',
                sys.executable, '-B', str(pathlib.Path(__file__).resolve()), *sys.argv[1:]]
        os.execvpe('unshare', argv, env)
    root = pathlib.Path('/mnt')
    sh('mount', '--make-rprivate', '/')
    sh('mount', '-t', 'tmpfs', '-o', 'size=512m,mode=755', 'none', root)
    for name in ('usr', 'etc', 'bin', 'sbin', 'lib', 'lib32', 'lib64', 'libx32', 'opt', 'var',
                 'home', 'dev', 'proc'):
        host = pathlib.Path('/', name)
        if not host.exists():
            continue
        if host.is_symlink():
            (root / name).symlink_to(os.readlink(host))
            continue
        (root / name).mkdir()
        sh('mount', '--rbind', host, root / name)
        if name not in ('home', 'dev', 'proc'):
            sh('mount', '-o', 'remount,bind,ro', root / name)
    (root / 'tmp').mkdir()
    sh('mount', '-t', 'tmpfs', '-o', 'size=512m,mode=1777', 'none', root / 'tmp')
    os.chroot(root)
    os.chdir(ROOT)
    sh('ip', 'link', 'set', 'lo', 'up')
    for address in (FANG_HOST, FLOOD_SOURCE, LEGIT_SOURCE):
        sh('ip', 'addr', 'add', f'{address}/32', 'dev', 'lo')


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('scenarios', nargs='+', help='e.g. idle:quic source:tcp')
    parser.add_argument('--idle', type=int, default=65)
    parser.add_argument('--active', type=int, default=30)
    parser.add_argument('--out', required=True)
    parser.add_argument('--keep-temp', action='store_true')
    args = parser.parse_args()
    if any(s.startswith('source:') for s in args.scenarios):
        enter_netns()

    results = []
    for scenario in args.scenarios:
        mode, kind = scenario.split(':')
        os.umask(0o077)
        lab = FloodLab(args.keep_temp)
        try:
            if mode == 'idle':
                result = scenario_idle(lab, kind, args.idle)
            elif mode == 'source':
                result = scenario_source(lab, kind, args.active)
            else:
                raise SystemExit(f'unknown scenario {scenario}')
        except LogCap as exc:
            result = {'scenario': scenario, 'log_cap': str(exc),
                      'capped': getattr(lab, 'capped', None)}
        except Exception as exc:  # record and continue with the next scenario
            result = {'scenario': scenario, 'error': f'{type(exc).__name__}: {exc}'}
        finally:
            result['daemon_sha256'] = getattr(lab, 'binary_hash', None)
            try:
                lab.cleanup()
            except Exception as exc:
                result['cleanup_error'] = str(exc)
        print(json.dumps(result), flush=True)
        results.append(result)
    pathlib.Path(args.out).write_text(json.dumps(results, indent=1) + '\n')


if __name__ == '__main__':
    main()
