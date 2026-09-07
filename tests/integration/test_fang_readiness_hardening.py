"""Characterization of the corrected Fang listener readiness contract."""
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class FangReadinessHardening(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((ROOT / "tests/fixtures/fang_readiness_hardening.json").read_text())

    def test_open_waits_for_listener_bind(self):
        self.assertTrue(self.data["readiness"]["open_waits_for_bind"])
        self.assertTrue(self.data["readiness"]["bind_failure_is_synchronous"])

    def test_busy_address_contract_is_preserved(self):
        self.assertEqual(
            self.data["readiness"]["duplicate_busy_address_error"],
            "FANG_ALREADY_ACTIVE",
        )


if __name__ == "__main__":
    unittest.main()
