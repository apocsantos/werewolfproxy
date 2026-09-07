import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class TargetAuthorizationMatrix(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((ROOT / "tests/fixtures/target_authorization_matrix.json").read_text())

    def test_deny_by_default_and_explicit_legacy_mode(self):
        self.assertEqual(self.data["default_mode"], "deny-by-default")
        self.assertEqual(self.data["schema"]["mode"], ["deny-by-default", "legacy-allow"])

    def test_ip_port_and_dns_rules_are_explicit(self):
        self.assertIn("authorized IPv4", self.data["cases"])
        self.assertIn("hostname mixed unauthorized", self.data["cases"])
        self.assertIn("zero target attempts", self.data["cases"])
        self.assertIn("require every address", self.data["resolution"])

    def test_plain_tcp_remains_outside_matrix(self):
        source = (ROOT / "crates/werewolfd/src/transport/tcp_plain.rs").read_text()
        self.assertNotIn("target_policy", source)

    def test_receiver_connects_only_resolved_endpoints_under_shared_deadline(self):
        # Source characterization supplements injected-resolver Rust tests.
        for name in ('tcp_encrypted.rs', 'quic.rs'):
            source = (ROOT / 'crates/werewolfd/src/transport' / name).read_text()
            start = source.index('let target_deadline =')
            connect = source.index('TcpStream::connect(authorized_targets.as_slice())', start)
            authorization = source.index('target_policy::authorize(', start)
            self.assertLess(authorization, connect)
            segment = source[start:connect]
            self.assertEqual(segment.count('timeout_at('), 2)
            self.assertEqual(segment.count('target_deadline,'), 2)
            self.assertEqual(segment.count('Duration::from_secs(5)'), 1)
            self.assertNotIn('lookup_host', source)
            self.assertNotIn('TcpStream::connect(remote)', source)
            self.assertNotIn('TcpStream::connect(&target)', source)
            ack = 'hs::TcpAck::new(' if name == 'tcp_encrypted.rs' else '\"ok\": true'
            self.assertGreater(source.index(ack, connect), connect)

    def test_resolution_has_one_dns_call_and_never_connects(self):
        source = (ROOT / 'crates/werewolfd/src/target_policy.rs').read_text().split('#[cfg(test)]')[0]
        self.assertEqual(source.count('tokio::net::lookup_host('), 1)
        self.assertNotIn('TcpStream', source)

    def test_lab_provisions_explicit_grants_not_legacy(self):
        source = (ROOT / 'tests/integration/run.py').read_text()
        self.assertNotIn('legacy-allow', source)
        self.assertIn("'mode': 'deny-by-default'", source)
        self.assertIn("identities['b']['fingerprint']: {'targets':", source)


if __name__ == "__main__":
    unittest.main()
