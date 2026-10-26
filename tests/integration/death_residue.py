#!/usr/bin/env python3
"""Stage30 media residue measurement (`docs/STAGE30_LIFECYCLE_SPEC.md`
section 10). Measured and reported, never asserted as erasure.

Run as root through sudo (the daemon itself runs as the invoking user):

    sudo python3 tests/integration/death_residue.py [BINARY_DIR]

BINARY_DIR defaults to target/stage1-lab/debug (built by any lab gate). The
script makes a 64 MiB ext4 image in a new private temporary directory,
loop-mounts it, runs one disposable daemon with its Den on it, gives it a
Pelt and a Pack member, retires it (the same destruction as death),
unmounts, and scans the raw image for the Pelt secret and the Pack key in
raw, hex and base64 forms. It reports what it found and deletes the image.
Nothing outside the new temporary directory is touched.
"""
import base64
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]


def run(*argv, user=None, check=True, **kwargs):
    if user:
        argv = ('runuser', '-u', user, '--', *argv)
    return subprocess.run([str(a) for a in argv], check=check, capture_output=True,
                          text=True, **kwargs)


def main():
    if os.geteuid() != 0 or 'SUDO_USER' not in os.environ:
        sys.exit('run with sudo: the image is loop-mounted as root, the daemon runs as you')
    user = os.environ['SUDO_USER']
    binaries = pathlib.Path(sys.argv[1] if len(sys.argv) > 1
                            else ROOT / 'target/stage1-lab/debug').resolve()
    daemon, ctl = binaries / 'werewolfd', binaries / 'werewolfctl'
    for binary in (daemon, ctl):
        if not binary.is_file():
            sys.exit(f'missing {binary}: run a lab gate first, or pass BINARY_DIR')

    work = pathlib.Path(tempfile.mkdtemp(prefix='wwp-residue-'))
    image, mount = work / 'den.img', work / 'mnt'
    mount.mkdir()
    mounted = False
    process = None
    try:
        run('truncate', '-s', '64M', image)
        run('mkfs.ext4', '-q', '-F', image)
        run('mount', '-o', 'loop', image, mount)
        mounted = True
        os.chmod(mount, 0o755)
        den, sock = mount / 'den', mount / 'run'
        for path in (den, sock):
            path.mkdir(mode=0o700)
            run('chown', user, path)
        # The scratch directory's ancestors must be safe for the Den check.
        os.chmod(work, 0o755)
        socket_path = sock / 'control.sock'
        base = [ctl, '--json', '--socket', socket_path]
        process = subprocess.Popen(
            ['runuser', '-u', user, '--', str(daemon), '--home', str(den), '--socket',
             str(socket_path), '--listen', '127.0.0.1:0', '--quic-listen', '127.0.0.1:0'],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(100):
            if socket_path.exists():
                break
            time.sleep(0.1)
        run(*base, 'pelt', 'init', user=user)
        run(*base, 'state', 'manifest-migrate', user=user)
        member = json.loads(run(*base, 'pelt', 'show', user=user).stdout)['result']
        # A Pack member whose key we can look for afterwards.
        peer_key = json.loads(run(ctl, '--json', 'governance', 'keygen',
                                  work / 'peer.key').stdout)['result']['public_key_b64']
        run(*base, 'pack', 'add', 'residue-peer', peer_key, 'quic://127.0.0.1:9', user=user)
        pelt = json.loads((den / 'pelt.json').read_text())
        secret = next(v for k, v in pelt.items() if 'secret' in k)

        needles = {}
        for label, b64 in (('pelt_secret', secret), ('pack_key', peer_key)):
            raw = base64.b64decode(b64)
            needles[label] = {'raw': raw, 'hex': raw.hex().encode(), 'base64': b64.encode()}

        run(*base, 'den', 'retire', '--confirm', member['fingerprint'], user=user)
        code = process.wait(timeout=30)
        process = None
        remaining = sorted(p.name for p in den.iterdir())
        run('sync')
        run('umount', mount)
        mounted = False

        data = image.read_bytes()
        found = {label: {form: data.count(needle) for form, needle in forms.items()}
                 for label, forms in needles.items()}
        report = {
            'filesystem': 'ext4 (mkfs defaults, data=ordered), loop image, 64 MiB',
            'daemon': str(daemon), 'exit': code, 'den_after': remaining,
            'occurrences_in_raw_image': found,
            'reading': 'counts are a measurement of this filesystem on this run, not a claim; '
                       '0 means "not found on this run", never "erased"',
        }
        print(json.dumps(report, indent=2))
    finally:
        if process is not None:
            process.kill()
        if mounted:
            run('umount', mount, check=False)
        run('rm', '-rf', work, check=False)


if __name__ == '__main__':
    main()
