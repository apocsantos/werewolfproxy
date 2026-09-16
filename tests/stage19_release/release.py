#!/usr/bin/env python3
"""Disposable Stage19 Linux release-archive and installer checks."""
import argparse
import hashlib
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

sys.dont_write_bytecode = True


EXPECTED_FILES = {
    'INSTALL.md',
    'RELEASE-METADATA',
    'SHA256SUMS',
    'bin/werewolfctl',
    'bin/werewolfd',
    'install.sh',
    'systemd/werewolfd.service',
    'uninstall.sh',
}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(argv, *, env=None, cwd=None, expected=0):
    completed = subprocess.run([str(arg) for arg in argv], cwd=cwd, env=env,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, timeout=30)
    if completed.returncode != expected:
        raise RuntimeError(
            f'{argv[0]} exit={completed.returncode}, expected={expected}: '
            f'{completed.stdout[-2000:]}{completed.stderr[-2000:]}')
    return completed


def mode(path):
    return stat.S_IMODE(path.stat().st_mode)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=pathlib.Path, required=True)
    parser.add_argument('--keep-temp', action='store_true')
    args = parser.parse_args()
    archive = args.archive.resolve()
    require(archive.is_file(), f'archive missing: {archive}')

    base = pathlib.Path(tempfile.mkdtemp(prefix='werewolf-stage19-release-'))
    os.chmod(base, 0o700)
    try:
        with tarfile.open(archive, 'r:gz') as package:
            members = package.getmembers()
            root_names = {member.name.split('/', 1)[0] for member in members}
            require(len(root_names) == 1, f'archive has unexpected roots: {root_names}')
            root_name = root_names.pop()
            require(root_name.startswith('werewolfproxy-'), 'archive root naming is invalid')
            package.extractall(base, filter='data')
        release = base / root_name
        files = {path.relative_to(release).as_posix() for path in release.rglob('*') if path.is_file()}
        require(files == EXPECTED_FILES, f'unexpected release inventory: {sorted(files)}')
        run(['sha256sum', '-c', 'SHA256SUMS'], cwd=release)
        print('PASS archive inventory and SHA256SUMS')

        stage = base / 'stage-root'
        environment = dict(os.environ, DESTDIR=str(stage))
        first = run([release / 'install.sh'], env=environment, cwd=release)
        require('Identity was NOT created.' in first.stdout, 'installer did not state identity noncreation')
        daemon = stage / 'usr/local/bin/werewolfd'
        control = stage / 'usr/local/bin/werewolfctl'
        unit = stage / 'usr/lib/systemd/system/werewolfd.service'
        state = stage / 'var/lib/werewolfproxy'
        require(daemon.is_file() and control.is_file() and unit.is_file() and state.is_dir(),
                'staged layout is incomplete')
        require(mode(daemon) == 0o755 and mode(control) == 0o755 and mode(unit) == 0o644,
                'staged binary or unit mode differs from release contract')
        require(mode(state) == 0o700 and not any(state.iterdir()),
                'installer created state or unsafe state mode')
        print('PASS staging install layout and no implicit identity')

        sentinel = state / 'synthetic-state-sentinel'
        sentinel.write_text('installer-must-not-rewrite-persistent-state\n')
        os.chmod(sentinel, 0o600)
        before = sha256(sentinel)
        run([release / 'install.sh'], env=environment, cwd=release)
        require(sha256(sentinel) == before and mode(state) == 0o700,
                'idempotent reinstall altered persistent state')
        print('PASS idempotent reinstall preserves staged Den')

        before_daemon = sha256(daemon)
        tampered = base / 'tampered-release'
        shutil.copytree(release, tampered)
        with (tampered / 'bin/werewolfctl').open('ab') as output:
            output.write(b'not-a-release-artifact')
        run([tampered / 'install.sh'], env=environment, cwd=tampered, expected=1)
        require(sha256(daemon) == before_daemon and not list(daemon.parent.glob('*.new.*')),
                'failed candidate replacement changed or left a partial daemon executable')
        print('PASS checksum failure preserves complete installed binary')

        victim = base / 'victim'
        victim.mkdir()
        unsafe = base / 'unsafe-root'
        unsafe.symlink_to(victim, target_is_directory=True)
        unsafe_env = dict(os.environ, DESTDIR=str(unsafe))
        run([release / 'install.sh'], env=unsafe_env, cwd=release, expected=1)
        require(not any(victim.iterdir()), 'installer traversed a symlinked staging root')
        print('PASS unsafe staging path rejected')

        empty = base / 'outside-source'
        empty.mkdir()
        require(run([daemon, '--version'], cwd=empty).stdout.strip(),
                'installed daemon did not execute outside source checkout')
        require(run([control, '--version'], cwd=empty).stdout.strip(),
                'installed control binary did not execute outside source checkout')
        print('PASS installed binaries execute without checkout')

        removed = run([release / 'uninstall.sh'], env=environment, cwd=release)
        require('Persistent Den retained' in removed.stdout, 'uninstaller did not state Den preservation')
        require(not daemon.exists() and not control.exists() and not unit.exists(),
                'uninstaller retained software artifact')
        require(sentinel.read_text() == 'installer-must-not-rewrite-persistent-state\n',
                'uninstaller removed persistent state')
        print('PASS uninstall removes software and retains Den')
    finally:
        if args.keep_temp:
            print(f'kept {base}')
        else:
            shutil.rmtree(base)
    return 0


if __name__ == '__main__':
    sys.exit(main())
