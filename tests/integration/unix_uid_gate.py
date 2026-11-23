#!/usr/bin/env python3
"""Run the SO_PEERCRED two-UID test in a private user namespace if available."""
import subprocess
import sys

sys.dont_write_bytecode = True

COMMAND = [
    'unshare', '--user', '--map-root-user', '--map-auto',
    '/bin/bash', '-lc',
    'umask 0002 && WEREWOLF_REQUIRE_UID_TEST=1 cargo test --locked --offline '
    '-p werewolfd --bin werewolfd '
    'unix_peer_credentials_distinguish_kernel_uids_when_root -- --nocapture',
]


def main():
    try:
        result = subprocess.run(COMMAND, capture_output=True, text=True, timeout=300)
    except FileNotFoundError:
        print('SKIP  unshare is unavailable')
        return 0
    except subprocess.TimeoutExpired:
        print('FAIL  multi-UID namespace test timed out')
        return 1

    if result.returncode == 0:
        print(result.stdout, end='')
        print('PASS  SO_PEERCRED distinguished UID 1 from UID 0')
        return 0

    detail = result.stderr + result.stdout
    if any(reason in detail.lower() for reason in (
        'operation not permitted',
        'write to uid_map failed',
        'write to gid_map failed',
        'newuidmap: failed',
    )):
        print('SKIP  host does not permit a private multi-UID user namespace')
        return 0

    print(detail, file=sys.stderr, end='')
    print('FAIL  multi-UID namespace test failed', file=sys.stderr)
    return 1


if __name__ == '__main__':
    raise SystemExit(main())
