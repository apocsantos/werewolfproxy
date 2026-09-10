"""Local selector authorization: deterministic probe counts, no live services."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class SelectorSecurity(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name)
        curl = self.base / 'curl'
        curl.write_text('''#!/usr/bin/env python3
import os, sys
with open(os.environ['PROBES'], 'a') as f: f.write(sys.argv[-1] + '\\n')
sys.exit(0 if sys.argv[-1] in os.environ['HEALTHY'].split(',') else 22)
''')
        curl.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.base) + ':' + os.environ['PATH'],
                        PROBES=str(self.base / 'probes'), HEALTHY='',
                        WEREWOLF_QUIC_URL='quic', WEREWOLF_TCP_ENC_URL='encrypted',
                        WEREWOLF_TCP_URL='plain', WEREWOLF_TRANSPORT_POLICY='secure')

    def select(self, args=(), healthy=()):
        self.env['HEALTHY'] = ','.join(healthy)
        (self.base / 'probes').write_text('')
        result = subprocess.run(['bash', str(ROOT / 'scripts/wolf-b.sh'), 'auto', '--json', *args],
                                env=self.env, capture_output=True, text=True, timeout=10)
        value = json.loads(result.stdout)
        self.assertEqual(result.returncode, 0 if value['healthy'] else 2)
        self.assertEqual(value['schema_version'], 2)
        self.assertEqual(value['fail_closed'], not value['healthy'])
        self.assertEqual(value['plaintext'], value['transport'] == 'tcp-plain')
        self.assertEqual(value['security_downgrade'], value['plaintext'])
        self.assertEqual(value['fallback'], value['transport'] in ('tcp-encrypted-v2', 'tcp-plain'))
        if value['policy_class'] == 'strict':
            self.assertFalse(value['security_downgrade'])
        return value, (self.base / 'probes').read_text().splitlines()

    def test_strict_order_and_zero_plain_probes(self):
        for policy in ('secure', 'strict'):
            for healthy, expected, probes in [(('quic', 'encrypted', 'plain'), 'quic', ['quic']),
                                              (('encrypted', 'plain'), 'tcp-encrypted-v2', ['quic', 'encrypted']),
                                              (('plain',), 'unavailable', ['quic', 'encrypted']),
                                              ((), 'unavailable', ['quic', 'encrypted'])]:
                with self.subTest(policy=policy, healthy=healthy):
                    value, actual = self.select(['--policy', policy], healthy)
                    self.assertEqual(value['transport'], expected)
                    self.assertEqual(actual, probes)

    def test_default_is_strict_even_with_plain_flag(self):
        value, probes = self.select(['--allow-plain-fallback'], ['plain'])
        self.assertEqual(value['policy_class'], 'strict')
        self.assertEqual(value['transport'], 'unavailable')
        self.assertEqual(probes, ['quic', 'encrypted'])

    def test_compatibility_authorization_is_invocation_scoped(self):
        for authorized in (False, True, False):
            args = ['--policy', 'compatibility'] + (['--allow-plain-fallback'] if authorized else [])
            value, probes = self.select(args, ['plain'])
            self.assertEqual(value['transport'], 'tcp-plain' if authorized else 'unavailable')
            self.assertTrue(value['plaintext_authorization_required'])
            self.assertEqual(value['plaintext_authorized'], authorized)
            self.assertEqual(probes, ['quic', 'encrypted'] + (['plain'] if authorized else []))

    def test_legacy_explicit_fallback(self):
        value, probes = self.select(['--policy', 'legacy'], ['plain'])
        self.assertEqual(value['transport'], 'tcp-plain')
        self.assertEqual(value['policy_class'], 'legacy')
        self.assertEqual(value['security_class'], 'plaintext')
        self.assertFalse(value['plaintext_authorization_required'])
        self.assertEqual(probes, ['quic', 'encrypted', 'plain'])

    def test_unknown_malformed_and_old_scoring_names_never_probe(self):
        for args in [['--policy'], ['--policy='], ['--policy', 'SECURE'], ['--bad'],
                     ['--policy', 'secure\"'], ['--policy', ' secure'],
                     *[['--policy', p] for p in ('unknown', 'learned', 'recent', 'resilience', 'performance', 'stealth')]]:
            with self.subTest(args=args):
                value, probes = self.select(args, ['quic', 'encrypted', 'plain'])
                self.assertEqual(value['transport'], 'unavailable')
                self.assertEqual(probes, [])

    def test_history_and_environment_cannot_authorize_plain(self):
        history = self.base / 'history'
        history.write_text(json.dumps({'transports': {'tcp-plain': {'score': 999999999},
                                                       'quic': {'score': -999999999}}}) + '\n')
        self.env.update(WEREWOLF_SCORE_FILE=str(history), WEREWOLF_SCORE_RECENT_SAMPLES='1',
                        WEREWOLF_ALLOW_PLAIN_FALLBACK='true')
        for policy in ('strict', 'secure', 'compatibility'):
            value, probes = self.select(['--policy', policy], ['plain'])
            self.assertEqual(value['transport'], 'unavailable')
            self.assertEqual(probes, ['quic', 'encrypted'])
        self.assertEqual(len(list(self.base.glob('history'))), 1)

    def test_restoration_no_cached_downgrade(self):
        for policy, flag in [('secure', []), ('compatibility', ['--allow-plain-fallback']), ('legacy', [])]:
            self.select(['--policy', policy, *flag], ['plain'])
            value, probes = self.select(['--policy', policy, *flag], ['quic', 'encrypted', 'plain'])
            self.assertEqual(value['transport'], 'quic')
            self.assertEqual(probes, ['quic'])

    def test_url_json_escaping(self):
        self.env['WEREWOLF_QUIC_URL'] = 'url"\\value'
        value, _ = self.select(healthy=['url"\\value'])
        self.assertEqual(value['url'], 'url"\\value')
