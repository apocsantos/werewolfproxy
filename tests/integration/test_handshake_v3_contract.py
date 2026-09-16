"""Independent fixed byte contract for the approved LP transcripts."""
import json
import pathlib
import struct
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


def fields(value):
    data = bytes.fromhex(value)
    assert data[:7] == b'WWP-HS\0'
    data = data[7:]
    result = []
    while data:
        size, = struct.unpack('>I', data[:4])
        assert len(data) >= 4 + size
        result.append(data[4:4 + size])
        data = data[4 + size:]
    return result


class HandshakeV3Contract(unittest.TestCase):
    def test_open_domains_and_fields(self):
        data = json.loads((ROOT / 'tests/fixtures/handshake_v3_transcripts.json').read_text())
        tcp, quic = fields(data['tcp_open']), fields(data['quic_open'])
        self.assertEqual(tcp[:2], [b'fang.tcp.open/v3', b'fang-tcp-v3'])
        self.assertEqual(quic[:2], [b'fang.quic.open/v3', b'fang-quic-v3'])
        self.assertEqual([len(x) for x in tcp[2:]], [28, 32, 28, 32, 16, 32, 14])
        self.assertEqual([len(x) for x in quic[2:]], [28, 28, 16, 14, 32])

    def test_ack_embeds_complete_open(self):
        data = json.loads((ROOT / 'tests/fixtures/handshake_v3_transcripts.json').read_text())
        tcp, quic = fields(data['tcp_ack']), fields(data['quic_ack'])
        self.assertEqual(tcp[:4], [b'fang.tcp.ack/v3', b'fang-tcp-v3', b'\1', b'fang-v3-secure'])
        self.assertEqual(tcp[4], bytes.fromhex(data['tcp_open']))
        self.assertEqual(quic[:3], [b'fang.quic.ack/v3', b'fang-quic-v3', b'\1'])
        self.assertEqual(quic[3], bytes.fromhex(data['quic_open']))
