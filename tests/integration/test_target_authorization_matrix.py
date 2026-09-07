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


if __name__ == "__main__":
    unittest.main()
