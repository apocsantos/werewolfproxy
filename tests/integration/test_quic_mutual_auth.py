"""Characterize the approved QUIC mutual-authentication protocol shape."""
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class QuicMutualAuthCharacterization(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((ROOT / "tests/fixtures/quic_mutual_auth_characterization.json").read_text())

    def test_versioned_transcripts_are_domain_separated(self):
        self.assertEqual(
            self.data["request_signed_template"],
            "fang.quic.open|fang-quic-v2|{sender_fingerprint}|{nonce}|{remote}",
        )
        self.assertEqual(
            self.data["ack_signed_template"],
            "fang.quic.ack|fang-quic-v2|{receiver_fingerprint}|{sender_fingerprint}|{nonce}|{remote}",
        )

    def test_ack_requires_complete_structure(self):
        self.assertEqual(
            self.data["ack_required_fields"],
            ["ok", "protocol", "receiver_pubkey", "receiver_fingerprint", "sender_fingerprint", "nonce", "remote", "signature"],
        )

    def test_negative_cases_are_explicit(self):
        self.assertIn("legacy plausible application bytes", self.data["negative_cases"])
        self.assertIn("replayed ack", self.data["negative_cases"])
        self.assertIn("target connection failure", self.data["negative_cases"])


if __name__ == "__main__":
    unittest.main()
