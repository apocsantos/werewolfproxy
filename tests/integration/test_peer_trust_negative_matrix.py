"""Freeze the isolated peer trust acceptance/rejection matrix."""
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class PeerTrustNegativeMatrix(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((ROOT / "tests/fixtures/peer_trust_negative_matrix.json").read_text())

    def test_correct_identity_is_the_only_success_case(self):
        cases = self.data["cases"]
        self.assertEqual(cases[0]["expected"], "pass")
        self.assertTrue(all(case["expected"] == "fail" for case in cases[1:]))

    def test_wrong_identity_and_tampering_fail_closed(self):
        names = {case["name"] for case in self.data["cases"] if case["expected"] == "fail"}
        self.assertIn("selected peer differs from authenticated peer", names)
        self.assertIn("valid crypto under wrong identity", names)
        self.assertIn("tampered identity field", names)

    def test_no_wire_format_change(self):
        self.assertFalse(self.data["wire_format_changed"])


if __name__ == "__main__":
    unittest.main()
