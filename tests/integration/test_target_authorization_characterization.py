import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class TargetAuthorizationCharacterization(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((ROOT / "tests/fixtures/target_authorization_characterization.json").read_text())

    def test_authenticated_peer_was_unrestricted(self):
        self.assertIn("any reachable target", self.data["pre_stage9"])
        self.assertEqual(set(self.data["transports"]), {"tcp-encrypted-v2", "quic"})

    def test_plain_tcp_is_not_receiver_authorized(self):
        self.assertTrue(self.data["plain_tcp_out_of_scope"])


if __name__ == "__main__":
    unittest.main()
