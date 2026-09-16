"""Characterize Stage 5 Fang lifecycle behavior before hardening."""
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class FangLifecycleDefects(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = ROOT / "tests" / "fixtures" / "fang_lifecycle_defects.json"
        cls.data = json.loads(path.read_text())

    def test_close_only_aborts_tracked_listener(self):
        self.assertTrue(self.data["close"]["listener_abort"])
        self.assertFalse(self.data["close"]["child_sessions_terminate"])

    def test_revoke_removes_records_without_stopping_work(self):
        revoke = self.data["pack_revoke"]
        self.assertTrue(revoke["records_removed"])
        self.assertFalse(revoke["tracked_listener_aborted"])
        self.assertFalse(revoke["child_sessions_terminate"])

    def test_silver_only_aborts_tracked_listeners(self):
        silver = self.data["silver"]
        self.assertTrue(silver["tracked_listener_aborted"])
        self.assertFalse(silver["child_sessions_terminate"])
        self.assertFalse(silver["reset_resurrects_tasks"])

    def test_open_does_not_wait_for_bind(self):
        readiness = self.data["readiness"]
        self.assertFalse(readiness["open_waits_for_bind"])
        self.assertFalse(readiness["bind_failure_is_synchronous"])


if __name__ == "__main__":
    unittest.main()
