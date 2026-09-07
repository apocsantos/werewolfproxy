"""Executable checks for the frozen Rust-side transport policy."""
import json
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


class TransportPolicyCharacterization(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = ROOT / "tests" / "fixtures" / "transport_policy_characterization.json"
        cls.data = json.loads(path.read_text())

    def test_requested_transport_defaults_and_aliases(self):
        values = self.data["requested_transport_inputs"]
        self.assertEqual(values["missing"], "quic")
        self.assertEqual(values["tcp"], "encrypted_tcp_v2")
        self.assertEqual(values["tcp-plain"], "plain_tcp")
        self.assertEqual(values["quic"], "quic")
        self.assertEqual(values["unknown"], "quic")

    def test_peer_address_parser_and_error_are_frozen(self):
        values = self.data["peer_address_inputs"]
        self.assertEqual(values["tcp://127.0.0.1:1"]["kind"], "tcp")
        self.assertEqual(values["quic://127.0.0.1:2"]["kind"], "quic")
        self.assertEqual(
            values["http://127.0.0.1:3"]["error"],
            "peer address must start with tcp:// or quic://",
        )

    def test_routing_topology_and_error_mapping(self):
        self.assertEqual(
            self.data["routing"],
            {
                "tcp": "remote_peer_encrypted_tcp_v2",
                "tcp-plain": "direct_target_plain_tcp",
                "quic": "remote_peer_quic",
            },
        )
        self.assertEqual(self.data["errors"]["tcp_with_invalid_peer"], "FANG_BAD_TCP_ADDRESS")
        self.assertEqual(self.data["errors"]["quic_with_invalid_peer"], "FANG_BAD_QUIC_ADDRESS")


if __name__ == "__main__":
    unittest.main()
