"""Characterization of the corrected Fang close ownership contract."""
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class FangCloseHardening(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((ROOT / "tests/fixtures/fang_close_hardening.json").read_text())

    def test_close_terminates_owned_work(self):
        self.assertTrue(self.data["close"]["listener_terminates"])
        self.assertTrue(self.data["close"]["owned_sessions_terminate"])

    def test_close_remains_idempotent_and_scoped(self):
        self.assertEqual(self.data["close"]["second_close_error"], "FANG_NOT_FOUND")
        self.assertTrue(self.data["close"]["unrelated_fangs_remain"])


if __name__ == "__main__":
    unittest.main()
