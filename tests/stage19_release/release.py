#!/usr/bin/env python3
"""Disposable Stage19 Linux release-archive and installer checks."""
import argparse
import hashlib
import json
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

sys.dont_write_bytecode = True

ROOT = pathlib.Path(__file__).resolve().parents[2]


def release_env():
    values = {}
    for line in (ROOT / 'packaging/release.env').read_text().splitlines():
        if '=' in line and not line.startswith('#'):
            key, value = line.split('=', 1)
            values[key] = value.strip().strip('"')
    return values


RELEASE = release_env()
SECURITY_RECORDS = set(RELEASE['SECURITY_RECORDS'].split())
EXPECTED_FILES = SECURITY_RECORDS | {
    'INSTALL.md',
    'README.md',
    'QUICKSTART.md',
    'RELEASE-MANIFEST.json',
    'RELEASE-METADATA',
    'RELEASE_NOTES.md',
    'SBOM.json',
    'SECURITY.md',
    'SHA256SUMS',
    'THIRD_PARTY_NOTICES',
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
        release_manifest = json.loads((release / 'RELEASE-MANIFEST.json').read_text())
        sbom = json.loads((release / 'SBOM.json').read_text())
        rustls, webpki = RELEASE['RUSTLS_VERSION'], RELEASE['RUSTLS_WEBPKI_VERSION']
        require(release_manifest['version'] == RELEASE['RELEASE_VERSION'], 'release manifest version mismatch')
        require(release_manifest['dependency_versions'] == {
            'rustls': rustls, 'rustls-webpki': webpki},
            'release manifest dependency versions mismatch')
        require(release_manifest['cargo_lock_sha256'] == sha256(ROOT / 'Cargo.lock'),
                'release manifest Cargo.lock digest mismatch')
        require(release_manifest['artifact_sha256']['bin/werewolfd'] == sha256(release / 'bin/werewolfd'),
                'release manifest daemon hash mismatch')
        require(sbom['cargo_lock_sha256'] == release_manifest['cargo_lock_sha256'],
                'SBOM lock digest differs from release manifest')
        components = {(item['name'], item['version']): item for item in sbom['components']}
        require(('rustls', rustls) in components and
                components[('rustls', rustls)]['included_in_linux_release'],
                'SBOM omits patched rustls from release closure')
        require(('rustls-webpki', webpki) in components and
                components[('rustls-webpki', webpki)]['included_in_linux_release'],
                'SBOM omits patched rustls-webpki from release closure')
        security_record = (release / 'docs/STAGE20S_RUSTLS_2026_0285_REMEDIATION.md').read_text()
        require('RUSTSEC-2026-0285' in security_record and 'rustls 0.23.45' in security_record,
                'archive omits Stage20S security history')
        if 'docs/STAGE33_RUSTLS_0_23_46_REMEDIATION.md' in SECURITY_RECORDS:
            security_record = (release / 'docs/STAGE33_RUSTLS_0_23_46_REMEDIATION.md').read_text()
            require('GHSA-c8m5-c952-6cm4' in security_record and '0.23.46' in security_record,
                    'archive omits Stage33 security history')
        if 'docs/STAGE33_QUIC_TLS_DEPS_REMEDIATION.md' in SECURITY_RECORDS:
            security_record = (release / 'docs/STAGE33_QUIC_TLS_DEPS_REMEDIATION.md').read_text()
            require('RUSTSEC-2026-0341' in security_record and 'quinn-proto' in security_record
                    and '0.11.18' in security_record and RELEASE['RUSTLS_WEBPKI_VERSION'] in security_record,
                    'archive omits Stage33 QUIC and webpki security history')
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
        home_path = str(pathlib.Path.home()).encode()
        require(all(home_path not in path.read_bytes() for path in release.rglob('*') if path.is_file()),
                'release artifact retained a developer-home path')
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
