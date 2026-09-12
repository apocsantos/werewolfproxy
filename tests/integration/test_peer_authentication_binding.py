"""Characterize the current peer authentication binding."""
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class PeerAuthenticationBinding(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((ROOT / "tests/fixtures/peer_authentication_binding.json").read_text())

    def test_encrypted_tcp_validates_ack_but_not_selected_peer(self):
        tcp = self.data["encrypted_tcp_v2"]
        self.assertFalse(tcp["expected_peer_carried_to_client"])
        self.assertTrue(tcp["ack_receiver_signature_verified"])
        self.assertTrue(tcp["ack_receiver_fingerprint_matches_key"])
        self.assertFalse(tcp["ack_receiver_matches_selected_pack_peer"])
        self.assertTrue(tcp["valid_wrong_pack_peer_can_pass"])

    def test_existing_negative_checks_are_frozen(self):
        tcp = self.data["encrypted_tcp_v2"]
        self.assertTrue(tcp["unknown_sender_rejected"])
        self.assertTrue(tcp["invalid_signature_rejected"])
        self.assertTrue(tcp["nonce_mismatch_rejected"])

    def test_quic_authenticates_selected_receiver_then_sender(self):
        quic = self.data["quic"]
        self.assertEqual(
            quic["authenticated_sides"],
            ["receiver_transport", "sender_application"],
        )
        self.assertEqual(
            quic["receiver_identity_proof"],
            "TLS1.3 CertificateVerify over exact selected-peer Pelt SPKI",
        )
        self.assertTrue(quic["selected_peer_receiver_binding"])
        self.assertFalse(quic["stage10_application_wire_format_changed"])
        self.assertFalse(quic["fingerprint_only_fallback"])


if __name__ == "__main__":
    unittest.main()
