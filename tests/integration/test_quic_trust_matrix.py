"""Freeze QUIC receiver-authentication rejection requirements."""
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class QuicTrustMatrix(unittest.TestCase):
    def test_all_negative_cases_are_required_to_fail_closed(self):
        data = json.loads((ROOT / "tests/fixtures/quic_mutual_auth_characterization.json").read_text())
        required = {
            "wrong receiver", "tampered fingerprint", "tampered public key",
            "tampered nonce", "tampered sender", "tampered remote",
            "invalid signature", "replayed ack", "truncated ack", "malformed json",
            "legacy plausible application bytes", "target connection failure",
        }
        self.assertTrue(required.issubset(set(data["negative_cases"])))

    def test_ack_is_not_application_data(self):
        data = json.loads((ROOT / "tests/fixtures/quic_mutual_auth_characterization.json").read_text())
        self.assertIn("legacy plausible application bytes", data["negative_cases"])
        self.assertEqual(data["ack_protocol"], "fang-quic-v2")


if __name__ == "__main__":
    unittest.main()
