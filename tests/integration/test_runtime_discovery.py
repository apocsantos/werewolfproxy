"""Local socket discovery only; never invoke an installed client or service."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class RuntimeDiscovery(unittest.TestCase):
    def test_clients_use_private_runtime_and_require_configuration(self):
        with tempfile.TemporaryDirectory(prefix='wwp-runtime-clients-') as temporary:
            base = Path(temporary)
            client = base / 'werewolfctl'
            client.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
            client.chmod(0o700)
            environment = dict(os.environ, PATH=str(base) + os.pathsep + os.environ['PATH'])
            runtime = base / 'private runtime'
            for wolf in 'abcd':
                with self.subTest(wolf=wolf):
                    environment['XDG_RUNTIME_DIR'] = str(runtime)
                    command = ['bash', str(ROOT / f'scripts/wolf-{wolf}.sh'), 'pack', 'list']
                    result = subprocess.run(command, env=environment, capture_output=True,
                                            text=True, timeout=5)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout.splitlines(),
                                     ['--socket', str(runtime / f'werewolf-{wolf}/control.sock'),
                                      'pack', 'list'])
                    del environment['XDG_RUNTIME_DIR']
                    rejected = subprocess.run(command, env=environment, capture_output=True,
                                              text=True, timeout=5)
                    self.assertNotEqual(rejected.returncode, 0)
                    self.assertEqual(rejected.stdout, '')

    def test_startup_scripts_leave_socket_cleanup_to_validated_daemon(self):
        names = ['start-wolf-a', 'start-wolf-b', 'start-pack', 'start-pack-bg',
                 'start-multiwolf', 'stop-pack', 'systemd-restart', 'install-systemd-user']
        for name in names:
            text = (ROOT / f'scripts/{name}.sh').read_text()
            self.assertNotRegex(text, r'rm\s+-f[^\n]*(?:\.sock|\$socket)')
            self.assertNotRegex(text, r'/tmp/wolf-[abcd]\.sock')
        units = (ROOT / 'scripts/install-systemd-user.sh').read_text()
        self.assertEqual(units.count('RuntimeDirectoryMode=0700'), 2)
        self.assertEqual(units.count('UMask=0077'), 2)
