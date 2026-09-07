"""Failure-path tests for orchestration, independent of Werewolf binaries."""
import contextlib
import io
import json
import pathlib
import shutil
import socket
import sys
import tempfile
import threading
import unittest
from unittest import mock

sys.dont_write_bytecode = True
from run import Lab, control_request, main


class HarnessTests(unittest.TestCase):
    def test_main_failure_returns_nonzero_and_removes_state(self):
        instances = []
        def fail(lab):
            instances.append(lab)
            lab.spawn([sys.executable, '-c', 'import time; time.sleep(60)'], 'sleeper')
            raise RuntimeError('injected acceptance failure')
        output = io.StringIO()
        with mock.patch.object(Lab, 'run', fail), mock.patch.object(sys, 'argv', ['run.py']):
            with contextlib.redirect_stdout(output), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(), 1)
        result = json.loads(output.getvalue().splitlines()[-1])
        self.assertEqual(result['status'], 'FAIL')
        self.assertFalse(instances[0].base.exists())
        self.assertTrue(all(p.returncode is not None for p in instances[0].processes))

    def test_wrong_application_bytes_fail(self):
        lab = Lab()
        lab.ports['quic'] = 1  # No connection is made: command response is injected.
        try:
            with mock.patch.object(lab, 'command', return_value=b'wrong payload'):
                with self.assertRaisesRegex(RuntimeError, 'payload differs'):
                    lab.get_marker('quic')
        finally:
            lab.cleanup()

    def test_failure_exit_and_cleanup(self):
        lab = Lab()
        base = lab.base
        try:
            sleeper, _ = lab.spawn([sys.executable, '-c', 'import time; time.sleep(60)'], 'sleeper')
            with self.assertRaisesRegex(RuntimeError, 'exited 7'):
                lab.command([sys.executable, '-c', 'raise SystemExit(7)'], 'failure')
        finally:
            lab.cleanup()
        self.assertIsNotNone(sleeper.returncode)
        self.assertFalse(base.exists())

    def test_timeout_kills_process(self):
        lab = Lab()
        try:
            with self.assertRaisesRegex(RuntimeError, 'timed out'):
                lab.command([sys.executable, '-c', 'import time; time.sleep(60)'],
                            'timeout', timeout=.1)
            self.assertIsNotNone(lab.processes[-1].returncode)
        finally:
            lab.cleanup()

    def test_keep_temp_still_stops_processes(self):
        lab = Lab(keep=True)
        try:
            process, _ = lab.spawn([sys.executable, '-c', 'import time; time.sleep(60)'], 'sleeper')
            lab.cleanup()
            self.assertIsNotNone(process.returncode)
            self.assertTrue((lab.base / 'cleanup.json').is_file())
        finally:
            shutil.rmtree(lab.base, ignore_errors=True)

    def test_descendant_cleanup(self):
        lab = Lab()
        child_file = lab.base / 'child.pid'
        try:
            parent, _ = lab.spawn([sys.executable, '-c',
                'import subprocess,sys,time,pathlib; '
                'p=subprocess.Popen([sys.executable,"-c","import time; time.sleep(60)"]); '
                'pathlib.Path(sys.argv[1]).write_text(str(p.pid)); p.wait()', str(child_file)], 'tree')
            child = int(lab.wait_for(lambda: child_file.read_text(), 'child PID', parent))
            lab.cleanup()
            stat = pathlib.Path(f'/proc/{child}/stat')
            # A killed orphan can briefly remain a zombie awaiting host init reaping.
            self.assertTrue(not stat.exists() or stat.read_text().split()[2] == 'Z')
        finally:
            if lab.base.exists():
                lab.cleanup()

    def response_case(self, responder, pattern):
        with tempfile.TemporaryDirectory(prefix='wwp-control-', dir='/tmp') as directory:
            path = pathlib.Path(directory) / 'control.sock'
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(str(path))
                server.listen()
                def serve():
                    with server.accept()[0] as conn:
                        request = json.loads(conn.recv(65536))
                        conn.sendall(responder(request))
                thread = threading.Thread(target=serve)
                thread.start()
                try:
                    with self.assertRaisesRegex((RuntimeError, ValueError), pattern):
                        control_request(path, 'status', timeout=1)
                finally:
                    thread.join(timeout=2)
                    self.assertFalse(thread.is_alive())

    def test_control_error_is_failure(self):
        self.response_case(lambda r: (json.dumps({'id': r['id'], 'ok': False,
                                                 'error': {'code': 'FAILED'}}) + '\n').encode(), 'FAILED')

    def test_control_id_is_checked(self):
        self.response_case(lambda r: b'{"id":"wrong","ok":true,"result":{}}\n', 'ID mismatch')

    def test_malformed_control_is_failure(self):
        self.response_case(lambda r: b'not-json\n', 'Expecting value')

    def test_control_eof_is_failure(self):
        self.response_case(lambda r: b'', 'EOF')


if __name__ == '__main__':
    unittest.main()
