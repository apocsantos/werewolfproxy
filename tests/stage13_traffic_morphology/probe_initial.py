#!/usr/bin/env python3
"""Inspect one production QUIC ClientHello using public QUIC v1 Initial keys.

This optional analysis needs the already-installed Python cryptography package.
It never uses endpoint secrets, key logs, private keys, or application data.
"""

import argparse
import hashlib
import hmac
import os
import pathlib
import subprocess
import time

from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

from capture import (Lab, Session, Target, TargetTask, TcpRelay, UdpRelay,
                     application_workload, digest, quic_varint, require,
                     setup, tls_hello, write_json)

ROOT = pathlib.Path(__file__).resolve().parents[2]
V1_SALT = bytes.fromhex("38762cf7f55934b34d179ae6a4c80cadccbb7f0a")
NUMERIC_TRANSPORT_PARAMETERS = {1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14, 32}


def expand(secret, label, size):
    full_label = b"tls13 " + label
    info = size.to_bytes(2, "big") + bytes([len(full_label)]) + full_label + b"\x00"
    out, previous, counter = b"", b"", 1
    while len(out) < size:
        previous = hmac.new(secret, previous + info + bytes([counter]),
                            hashlib.sha256).digest()
        out += previous
        counter += 1
    return out[:size]


def take_varint(data, offset):
    value, end = quic_varint(data, offset)
    require(value is not None, "truncated public QUIC varint")
    return value, end


def inspect(datagram):
    require(len(datagram) >= 1200 and datagram[0] & 0xf0 == 0xc0,
            "not a QUIC v1 client Initial datagram")
    require(datagram[1:5] == b"\x00\x00\x00\x01", "not QUIC v1")
    dcid_len = datagram[5]
    dcid = datagram[6:6 + dcid_len]
    offset = 6 + dcid_len
    scid_len = datagram[offset]
    offset += 1 + scid_len
    token_len, offset = take_varint(datagram, offset)
    require(token_len == 0, "unexpected Initial token in one-shot production probe")
    offset += token_len
    protected_length, pn_offset = take_varint(datagram, offset)
    end = pn_offset + protected_length
    require(end <= len(datagram), "truncated Initial packet")

    initial_secret = hmac.new(V1_SALT, dcid, hashlib.sha256).digest()
    client_secret = expand(initial_secret, b"client in", 32)
    key = expand(client_secret, b"quic key", 16)
    iv = expand(client_secret, b"quic iv", 12)
    hp = expand(client_secret, b"quic hp", 16)
    sample = datagram[pn_offset + 4:pn_offset + 20]
    require(len(sample) == 16, "truncated header protection sample")
    mask = Cipher(algorithms.AES(hp), modes.ECB()).encryptor().update(sample)[:5]
    first = datagram[0] ^ (mask[0] & 0x0f)
    pn_len = (first & 3) + 1
    pn_bytes = bytes(datagram[pn_offset + i] ^ mask[i + 1] for i in range(pn_len))
    packet_number = int.from_bytes(pn_bytes, "big")
    header = bytes([first]) + datagram[1:pn_offset] + pn_bytes
    nonce = bytes(a ^ b for a, b in zip(iv, packet_number.to_bytes(12, "big")))
    plaintext = AESGCM(key).decrypt(
        nonce, datagram[pn_offset + pn_len:end], header)

    crypto = {}
    frame_types = []
    padding_bytes = 0
    offset = 0
    while offset < len(plaintext):
        frame_type, offset = take_varint(plaintext, offset)
        frame_types.append(frame_type)
        if frame_type == 0:
            padding_bytes += 1
        elif frame_type == 1:
            pass
        elif frame_type == 6:
            stream_offset, offset = take_varint(plaintext, offset)
            size, offset = take_varint(plaintext, offset)
            for index, byte in enumerate(plaintext[offset:offset + size]):
                crypto[stream_offset + index] = byte
            offset += size
        else:
            raise RuntimeError(f"unexpected public Initial frame type {frame_type}")
    hello = bytes(crypto[i] for i in range(len(crypto)))
    require(hello[0] == 1, "Initial CRYPTO does not begin with ClientHello")
    fake_record = b"\x16\x03\x01" + len(hello).to_bytes(2, "big") + hello
    parsed = tls_hello(fake_record, 1)
    require(parsed is not None, "could not parse public ClientHello")
    parsed.pop("legacy_record_version")  # QUIC has no TLS records.

    # Parse public ClientHello extension 57 (QUIC transport parameters) as
    # ID/length/value metadata. Never retain connection-ID byte values.
    body = memoryview(hello)[4:]
    cursor = 2 + 32
    cursor += 1 + body[cursor]
    suites_len = int.from_bytes(body[cursor:cursor + 2], "big")
    cursor += 2 + suites_len
    cursor += 1 + body[cursor]
    ext_len = int.from_bytes(body[cursor:cursor + 2], "big")
    cursor += 2
    ext_end = cursor + ext_len
    transport_parameters = []
    while cursor + 4 <= ext_end:
        kind = int.from_bytes(body[cursor:cursor + 2], "big")
        size = int.from_bytes(body[cursor + 2:cursor + 4], "big")
        value = bytes(body[cursor + 4:cursor + 4 + size])
        cursor += 4 + size
        if kind != 57:
            continue
        pos = 0
        while pos < len(value):
            parameter_id, pos = take_varint(value, pos)
            parameter_len, pos = take_varint(value, pos)
            raw = value[pos:pos + parameter_len]
            pos += parameter_len
            entry = {"id": parameter_id, "value_length": parameter_len}
            if parameter_id in NUMERIC_TRANSPORT_PARAMETERS and 0 < parameter_len <= 8:
                number, end_pos = quic_varint(raw, 0)
                if number is not None and end_pos == len(raw):
                    entry["public_varint_value"] = number
            transport_parameters.append(entry)
    return {
        "quic_version": "00000001", "datagram_bytes": len(datagram),
        "dcid_length": dcid_len, "scid_length": scid_len, "token_length": token_len,
        "initial_aead_authentication": "PASS",
        "initial_key_basis": "public QUIC v1 salt and client DCID only",
        "client_hello_bytes": len(hello), "client_hello": parsed,
        "transport_parameters": transport_parameters,
        "initial_padding_bytes": padding_bytes,
        "initial_nonpadding_frame_types": sorted(set(frame_types) - {0}),
    }


def main(args):
    os.umask(0o077)
    lab = Lab()
    target = Target()
    tcp = TcpRelay(("127.0.0.1", 1))
    udp = UdpRelay(tcp.port, ("127.0.0.1", 1))
    try:
        setup(lab, target, tcp, udp)
        tcp.upstream = ("127.0.0.1", lab.ports["a_tcp"])
        udp.upstream = ("127.0.0.1", lab.ports["a_quic"])
        samples = []
        for sample in range(args.samples):
            task = TargetTask("handshake_only")
            target.tasks.put(task)
            session = Session("quic", "handshake_only", sample)
            udp.first_initial = None
            udp.active = session
            application_workload(lab.ports["quic_fang"], task, session)
            require(task.accepted.wait(20), "production target not reached")
            time.sleep(.15)
            require(udp.first_initial is not None, "no production client Initial observed")
            samples.append(inspect(udp.first_initial))
            udp.active = None
        result = {
            "schema": 1, "sample_count": len(samples), "samples": samples,
            "source_head": subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            "lock_sha256": digest(ROOT / "Cargo.lock"),
            "production_path": "Fang -> selected Pack peer -> UDP relay -> production listener",
            "target_reached_after_authentication": True,
            "raw_initial_retained": False,
            "endpoint_secrets_or_keylog_used": False,
        }
        write_json(pathlib.Path(args.output), result)
        print(f"PRODUCTION_QUIC_PUBLIC_INITIAL_INSPECTION=PASS {len(samples)} samples")
    finally:
        tcp.close()
        udp.close()
        target.close()
        lab.cleanup()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", default="tests/stage13_traffic_morphology/initial_probe.json")
    parser.add_argument("--samples", type=int, default=10)
    main(parser.parse_args())
