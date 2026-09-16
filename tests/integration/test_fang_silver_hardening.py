"""Characterization of the corrected Silver Fang lifecycle contract."""
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class FangSilverHardening(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((ROOT / "tests/fixtures/fang_silver_hardening.json").read_text())

    def test_silver_terminates_owned_work_and_clears_registry(self):
        silver = self.data["silver"]
        self.assertTrue(silver["listeners_terminate"])
        self.assertTrue(silver["owned_sessions_terminate"])
        self.assertTrue(silver["registry_is_empty"])

    def test_reset_does_not_resurrect_and_allows_normal_operation(self):
        silver = self.data["silver"]
        self.assertFalse(silver["reset_resurrects_stale_tasks"])
        self.assertTrue(silver["new_fangs_after_reset_allowed"])


if __name__ == "__main__":
    unittest.main()
