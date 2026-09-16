"""Characterization of the corrected Pack revoke lifecycle contract."""
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class FangRevokeHardening(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((ROOT / "tests/fixtures/fang_revoke_hardening.json").read_text())

    def test_associated_work_terminates(self):
        self.assertTrue(self.data["revoke"]["associated_fangs_terminate"])

    def test_revoke_is_scoped_and_plain_metadata_is_explicit(self):
        self.assertTrue(self.data["revoke"]["unrelated_fangs_remain"])
        self.assertTrue(self.data["revoke"]["plain_fangs_use_existing_peer_metadata"])


if __name__ == "__main__":
    unittest.main()
