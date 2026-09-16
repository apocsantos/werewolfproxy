"""Executable checks for the frozen encrypted TCP v2 characterization fixture."""
import json
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


class EncryptedTcpCharacterization(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        fixture = ROOT / "tests" / "fixtures" / "tcp_encrypted_v2_characterization.json"
        cls.data = json.loads(fixture.read_text())

    def test_signed_transcript_templates(self):
        self.assertEqual(
            self.data["hello_signed_template"],
            "fang.pipe|{sender_fingerprint}|{remote}|{nonce}|{client_x25519}",
        )
        self.assertEqual(
            self.data["ack_signed_template"],
            "fang.ack|{receiver_fingerprint}|{sender_fingerprint}|{nonce}|{server_x25519}",
        )

    def test_directional_nonce_vectors(self):
        for vector in self.data["nonce_vectors"]:
            counter = vector["counter"].to_bytes(8, "big")
            nonce = bytes([vector["direction"], 0, 0, 0]) + counter
            self.assertEqual(nonce.hex(), vector["hex"])

    def test_frame_limits(self):
        limits = self.data["frame_limits"]
        self.assertEqual(limits["maximum_plaintext"] + 2, limits["maximum_frame_size"])
        self.assertEqual(limits["length_prefix_bytes"], 4)
        self.assertEqual(limits["aead_tag_bytes"], 16)
        self.assertEqual((limits["maximum_frame_size"] - limits["minimum_frame_size"]) % limits["step"], 0)


if __name__ == "__main__":
    unittest.main()
