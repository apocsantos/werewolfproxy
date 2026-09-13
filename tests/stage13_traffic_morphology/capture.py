#!/usr/bin/env python3
"""Production-path, metadata-only TCP/TLS and QUIC morphology capture.

The relay never terminates TLS, decrypts QUIC, or retains application payloads.
TCP recv() units are relay observations, not claims about original IP segments.
"""

import argparse
import csv
import hashlib
import json
import os
import pathlib
import queue
import socket
import subprocess
import sys
import threading
import time

sys.dont_write_bytecode = True
ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests" / "integration"))
from run import Lab, digest, require, write_json  # noqa: E402

BURST_GAP_MS = 20  # Fixed before measurement, not chosen from observations.
WORKLOADS = (
    "handshake_only", "tiny", "interactive", "request_1kib",
    "transfer_64kib", "transfer_1mib", "bulk_50mib",
    "asymmetric_upload", "asymmetric_download", "idle",
)
CHUNK = hashlib.sha256(b"stage13 deterministic application workload").digest() * 2048
INTERACTIVE = (16, 32, 64, 128, 256, 16, 64, 128)


def read_exact(sock, size):
    out = bytearray()
    while len(out) < size:
        data = sock.recv(min(65536, size - len(out)))
        require(data, f"application EOF after {len(out)}/{size} bytes")
        out.extend(data)
    return bytes(out)


def stream_hash(sock, size):
    result = hashlib.sha256()
    remaining = size
    first_ns = None
    while remaining:
        data = sock.recv(min(65536, remaining))
        require(data, f"application EOF with {remaining} bytes remaining")
        if first_ns is None:
            first_ns = time.monotonic_ns()
        result.update(data)
        remaining -= len(data)
    return result.hexdigest(), first_ns


def repeated_hash(size):
    result = hashlib.sha256()
    remaining = size
    while remaining:
        part = CHUNK[:min(remaining, len(CHUNK))]
        result.update(part)
        remaining -= len(part)
    return result.hexdigest()


class Session:
    def __init__(self, transport, workload, sample):
        self.id = f"{transport}-{workload}-{sample:02d}"
        self.transport, self.workload, self.sample = transport, workload, sample
        self.units = []
        self.records = []
        self.hello = {}
        self.initials = []
        self.lock = threading.Lock()
        self.app_first_send_ns = None
        self.app_first_response_ns = None
        self.target_accept_ns = None
        self.app_close_ns = None
        self.bridge_done = threading.Event()

    def unit(self, direction, size):
        with self.lock:
            self.units.append((time.monotonic_ns(), direction, size))


def tls_hello(raw, expected_type):
    """Parse only public TLS handshake fields from a complete plaintext record."""
    if len(raw) < 9 or raw[0] != 22 or raw[5] != expected_type:
        return None
    body_len = int.from_bytes(raw[6:9], "big")
    body = memoryview(raw)[9:9 + body_len]
    if len(body) != body_len or len(body) < 38:
        return None
    offset = 2 + 32
    sid_len = body[offset]
    offset += 1 + sid_len
    if expected_type == 1:
        suite_bytes = int.from_bytes(body[offset:offset + 2], "big")
        offset += 2
        suites = [body[i:i + 2].hex() for i in range(offset, offset + suite_bytes, 2)]
        offset += suite_bytes
        compression_len = body[offset]
        offset += 1 + compression_len
    else:
        suites = [body[offset:offset + 2].hex()]
        offset += 3
    if offset + 2 > len(body):
        return None
    ext_len = int.from_bytes(body[offset:offset + 2], "big")
    offset += 2
    end = offset + ext_len
    extensions = []
    groups, signatures, keyshares, versions = [], [], [], []
    while offset + 4 <= end and offset + 4 <= len(body):
        kind = int.from_bytes(body[offset:offset + 2], "big")
        length = int.from_bytes(body[offset + 2:offset + 4], "big")
        data = body[offset + 4:offset + 4 + length]
        if len(data) != length:
            return None
        extensions.append(kind)
        if expected_type == 1 and kind in (10, 13) and len(data) >= 2:
            values = [int.from_bytes(data[i:i + 2], "big")
                      for i in range(2, len(data), 2)]
            if kind == 10:
                groups = values
            else:
                signatures = values
        if expected_type == 1 and kind == 51 and len(data) >= 2:
            cursor = 2
            while cursor + 4 <= len(data):
                group = int.from_bytes(data[cursor:cursor + 2], "big")
                size = int.from_bytes(data[cursor + 2:cursor + 4], "big")
                keyshares.append({"group": group, "public_key_length": size})
                cursor += 4 + size
        if kind == 43:
            versions = list(bytes(data))
        offset += 4 + length
    return {
        "handshake_bytes": body_len + 4,
        "legacy_record_version": raw[1:3].hex(),
        "legacy_hello_version": bytes(body[:2]).hex(),
        "cipher_suites": suites,
        "extension_order": extensions,
        "supported_groups": groups,
        "signature_algorithms": signatures,
        "key_shares": keyshares,
        "supported_versions_raw": versions,
        "sni_present": 0 in extensions,
        "alpn_present": 16 in extensions,
    }


def quic_varint(data, pos):
    if pos >= len(data):
        return None, pos
    size = 1 << (data[pos] >> 6)
    if pos + size > len(data):
        return None, pos
    return int.from_bytes(data[pos:pos + size], "big") & ((1 << (8 * size - 2)) - 1), pos + size


def quic_public_header(data):
    if len(data) < 7 or data[0] & 0x80 == 0:
        return None
    kind = (data[0] >> 4) & 3
    pos = 5
    dcid_len = data[pos]
    pos += 1 + dcid_len
    if pos >= len(data):
        return None
    scid_len = data[pos]
    pos += 1 + scid_len
    if pos > len(data):
        return None
    token_len = None
    if kind == 0:
        token_len, pos = quic_varint(data, pos)
    return {
        "version": data[1:5].hex(), "long_packet_type": kind,
        "dcid_len": dcid_len, "scid_len": scid_len, "token_len": token_len,
        "datagram_len": len(data),
    }


class TcpRelay:
    def __init__(self, upstream):
        self.upstream = upstream
        self.listener = socket.socket()
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.listener.settimeout(.2)
        self.port = self.listener.getsockname()[1]
        self.active = None
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stop.is_set():
            try:
                client, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            session = self.active
            if session is None:
                client.close()
                continue
            threading.Thread(target=self.bridge, args=(client, session), daemon=True).start()

    def bridge(self, client, session):
        upstream = socket.create_connection(self.upstream, timeout=10)
        streams = {"c2s": bytearray(), "s2c": bytearray()}

        def forward(source, target, direction):
            try:
                while True:
                    data = source.recv(65536)
                    if not data:
                        break
                    session.unit(direction, len(data))
                    buf = streams[direction]
                    buf.extend(data)
                    while len(buf) >= 5:
                        record_len = int.from_bytes(buf[3:5], "big") + 5
                        if record_len > 18437:
                            raise RuntimeError("invalid TLS record length")
                        if len(buf) < record_len:
                            break
                        record = bytes(buf[:record_len])
                        del buf[:record_len]
                        with session.lock:
                            session.records.append((time.monotonic_ns(), direction,
                                                    len(record), record[0]))
                            name = "client_hello" if direction == "c2s" else "server_hello"
                            if name not in session.hello:
                                parsed = tls_hello(record, 1 if direction == "c2s" else 2)
                                if parsed is not None:
                                    session.hello[name] = parsed
                    target.sendall(data)
            finally:
                try:
                    target.shutdown(socket.SHUT_WR)
                except OSError:
                    pass

        c = threading.Thread(target=forward, args=(client, upstream, "c2s"))
        s = threading.Thread(target=forward, args=(upstream, client, "s2c"))
        c.start()
        s.start()
        c.join()
        s.join()
        client.close()
        upstream.close()
        session.bridge_done.set()

    def close(self):
        self.stop.set()
        self.listener.close()
        self.thread.join(timeout=2)


class UdpRelay:
    def __init__(self, port, upstream):
        self.upstream = upstream
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", port))
        self.sock.settimeout(.2)
        self.active = None
        self.client = None
        self.clients_by_cid = {}
        self.sessions_by_client = {}
        self.first_initial = None  # Public QUIC Initial, memory only for optional probe.
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stop.is_set():
            try:
                data, addr = self.sock.recvfrom(65536)
            except socket.timeout:
                continue
            except OSError:
                break
            direction = "s2c" if addr == self.upstream else "c2s"
            if direction == "c2s":
                ids = self.long_header_ids(data)
                session = self.sessions_by_client.get(addr)
                fresh_initial = ids is not None and ids[1] not in self.clients_by_cid
                if session is None or (fresh_initial and session is not self.active):
                    session = self.active
                    if session is None:
                        continue
                    self.sessions_by_client[addr] = session
                self.client = addr
                if ids is not None:
                    self.clients_by_cid[ids[1]] = addr
                destination = self.upstream
            else:
                ids = self.long_header_ids(data)
                dcid = ids[0] if ids is not None else data[1:9]
                destination = self.clients_by_cid.get(dcid, self.client)
                session = self.sessions_by_client.get(destination)
                if session is None:
                    session = self.active
            if destination is None:
                continue
            if session is not None:
                session.unit(direction, len(data))
            if direction == "c2s" and self.first_initial is None and data[0] & 0xf0 == 0xc0:
                self.first_initial = bytes(data)
            if direction == "c2s" and session is not None and len(session.initials) < 4:
                public = quic_public_header(data)
                if public:
                    with session.lock:
                        session.initials.append(public)
            self.sock.sendto(data, destination)

    @staticmethod
    def long_header_ids(data):
        if len(data) < 7 or data[0] & 0x80 == 0:
            return None
        dcid_len = data[5]
        pos = 6 + dcid_len
        if pos >= len(data):
            return None
        scid_len = data[pos]
        end = pos + 1 + scid_len
        if end > len(data):
            return None
        return data[6:pos], data[pos + 1:end]

    def close(self):
        self.stop.set()
        self.sock.close()
        self.thread.join(timeout=2)


class TargetTask:
    def __init__(self, workload):
        self.workload = workload
        self.accepted = threading.Event()
        self.done = threading.Event()
        self.accept_ns = None
        self.error = None


class Target:
    def __init__(self):
        self.sock = socket.socket()
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen()
        self.sock.settimeout(.2)
        self.port = self.sock.getsockname()[1]
        self.tasks = queue.Queue()
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stop.is_set():
            try:
                conn, _ = self.sock.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            task = self.tasks.get(timeout=10)
            threading.Thread(target=self.handle, args=(conn, task), daemon=True).start()

    def handle(self, conn, task):
        task.accept_ns = time.monotonic_ns()
        task.accepted.set()
        conn.settimeout(120)
        try:
            if task.workload in ("handshake_only", "idle"):
                while conn.recv(65536):
                    raise AssertionError("unexpected idle application bytes")
            elif task.workload == "asymmetric_upload":
                remaining = 1024 * 1024
                while remaining:
                    received = conn.recv(min(65536, remaining))
                    require(received, "upload truncated at target")
                    remaining -= len(received)
                conn.sendall(b"R" * 32)
            elif task.workload in ("asymmetric_download", "bulk_50mib"):
                require(read_exact(conn, 32) == b"Q" * 32, "download request mismatch")
                remaining = 4 * 1024 * 1024 if task.workload == "asymmetric_download" else 50 * 1024 * 1024
                while remaining:
                    part = CHUNK[:min(remaining, len(CHUNK))]
                    conn.sendall(part)
                    remaining -= len(part)
            else:
                while data := conn.recv(65536):
                    conn.sendall(data)
        except Exception as exc:
            task.error = type(exc).__name__
        finally:
            conn.close()
            task.done.set()

    def close(self):
        self.stop.set()
        self.sock.close()
        self.thread.join(timeout=2)


def application_workload(port, task, session):
    sock = socket.create_connection(("127.0.0.1", port), timeout=10)
    sock.settimeout(120)
    try:
        if task.workload in ("handshake_only", "idle"):
            require(task.accepted.wait(20), "target not reached after secure handshake")
            if task.workload == "idle":
                time.sleep(1.0)
        elif task.workload in ("asymmetric_download", "bulk_50mib"):
            session.app_first_send_ns = time.monotonic_ns()
            sock.sendall(b"Q" * 32)
            size = 4 * 1024 * 1024 if task.workload == "asymmetric_download" else 50 * 1024 * 1024
            actual, first = stream_hash(sock, size)
            session.app_first_response_ns = first
            require(actual == repeated_hash(size), "download integrity failure")
        elif task.workload == "asymmetric_upload":
            session.app_first_send_ns = time.monotonic_ns()
            for _ in range(16):
                sock.sendall(CHUNK)
            reply = read_exact(sock, 32)
            session.app_first_response_ns = time.monotonic_ns()
            require(reply == b"R" * 32, "upload receipt mismatch")
        elif task.workload == "interactive":
            for index, size in enumerate(INTERACTIVE):
                payload = CHUNK[:size]
                if index == 0:
                    session.app_first_send_ns = time.monotonic_ns()
                sock.sendall(payload)
                reply = read_exact(sock, size)
                if index == 0:
                    session.app_first_response_ns = time.monotonic_ns()
                require(reply == payload, "interactive echo mismatch")
        else:
            size = {"tiny": 32, "request_1kib": 1024,
                    "transfer_64kib": 65536, "transfer_1mib": 1024 * 1024}[task.workload]
            session.app_first_send_ns = time.monotonic_ns()
            if size <= len(CHUNK):
                sock.sendall(CHUNK[:size])
                reply = read_exact(sock, size)
                session.app_first_response_ns = time.monotonic_ns()
                require(reply == CHUNK[:size], "echo mismatch")
            else:
                writer = threading.Thread(target=lambda: [
                    sock.sendall(CHUNK) for _ in range(size // len(CHUNK))])
                writer.start()
                actual, first = stream_hash(sock, size)
                writer.join(timeout=20)
                require(not writer.is_alive(), "upload writer stuck")
                session.app_first_response_ns = first
                require(actual == repeated_hash(size), "echo hash mismatch")
    finally:
        try:
            sock.shutdown(socket.SHUT_WR)
        except OSError:
            pass
        sock.close()
        session.app_close_ns = time.monotonic_ns()


def features(session):
    units = sorted(session.units)
    counts = {d: sum(1 for _, side, _ in units if side == d) for d in ("c2s", "s2c")}
    totals = {d: sum(size for _, side, size in units if side == d) for d in ("c2s", "s2c")}
    lengths = [size for _, _, size in units]
    gaps_ms = [(units[i][0] - units[i - 1][0]) / 1e6 for i in range(1, len(units))]
    bursts = []
    for i, (_, _, size) in enumerate(units):
        if i == 0 or gaps_ms[i - 1] > BURST_GAP_MS:
            bursts.append(0)
        bursts[-1] += size
    ordered = sorted(lengths)
    hist_edges = (128, 256, 512, 768, 1024, 1200, 1500, 2048, 4096, 8192, 16384)
    histogram = {str(edge): sum(1 for x in lengths if x <= edge) for edge in hist_edges}
    histogram[">16384"] = sum(1 for x in lengths if x > 16384)
    return {
        "id": session.id, "transport": session.transport, "workload": session.workload,
        "sample": session.sample, "total_c2s": totals["c2s"], "total_s2c": totals["s2c"],
        "count_c2s": counts["c2s"], "count_s2c": counts["s2c"],
        "first_16": [{"direction": d, "wire_len": n} for _, d, n in units[:16]],
        "duration_ms": round((units[-1][0] - units[0][0]) / 1e6, 3) if len(units) > 1 else 0,
        "time_to_first_response_ms": round(
            (session.app_first_response_ns - session.app_first_send_ns) / 1e6, 3)
        if session.app_first_response_ns and session.app_first_send_ns else None,
        "secure_handshake_to_target_ms": round(
            (session.target_accept_ns - units[0][0]) / 1e6, 3)
        if units and session.target_accept_ns else None,
        "interarrival_median_ms": sorted(gaps_ms)[len(gaps_ms) // 2] if gaps_ms else None,
        "interarrival_p90_ms": sorted(gaps_ms)[int(.9 * (len(gaps_ms) - 1))] if gaps_ms else None,
        "burst_count": len(bursts), "burst_bytes": bursts[:32],
        "largest_unit": max(lengths, default=0), "smallest_nonempty_unit": min(lengths, default=0),
        "mean_unit": round(sum(lengths) / len(lengths), 3) if lengths else 0,
        "median_unit": ordered[len(ordered) // 2] if ordered else 0,
        "size_histogram_cumulative": histogram,
        "direction_changes": sum(units[i][1] != units[i - 1][1] for i in range(1, len(units))),
        "up_down_byte_ratio": round(totals["c2s"] / totals["s2c"], 4) if totals["s2c"] else None,
        "tls_record_count": len(session.records),
        "tls_record_lengths_first_16": [n for _, _, n, _ in session.records[:16]],
        "tls_record_types_first_16": [kind for _, _, _, kind in session.records[:16]],
        "tls_client_hello": session.hello.get("client_hello"),
        "tls_server_hello": session.hello.get("server_hello"),
        "quic_initial_headers": session.initials,
        "idle_units_after_target_accept": sum(
            1 for t, _, _ in units if session.target_accept_ns and t > session.target_accept_ns)
        if session.workload == "idle" else None,
    }


def retain_disposable_unit_metadata(session, directory):
    """Write all unit/record metadata, never payload, outside committed artifacts."""
    directory.mkdir(parents=True, exist_ok=True)
    units = sorted(session.units)
    first_ns = units[0][0]
    unit_path = directory / f"{session.id}.units.csv"
    with unit_path.open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(("session_id", "transport", "direction", "timestamp_us_from_first",
                         "wire_length", "unit_kind"))
        for timestamp, direction, size in units:
            writer.writerow((session.id, session.transport, direction,
                             (timestamp - first_ns) // 1000, size,
                             "udp_datagram" if session.transport == "quic" else "tcp_relay_recv"))
    record_path = directory / f"{session.id}.tls_records.csv"
    with record_path.open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(("session_id", "direction", "timestamp_us_from_first",
                         "record_length", "tls_content_type"))
        for timestamp, direction, size, kind in session.records:
            writer.writerow((session.id, direction, (timestamp - first_ns) // 1000, size, kind))
    return hashlib.sha256(unit_path.read_bytes()).hexdigest()


def setup(lab, target, tcp_relay, udp_relay):
    build_env = {k: v for k, v in os.environ.items()
                 if not k.startswith("WEREWOLF_") and k not in ("BASH_ENV", "ENV")}
    subprocess.run(["cargo", "build", "--locked", "--offline", "--workspace", "--bins",
                    "--target-dir", str(ROOT / "target" / "stage1-lab")],
                   cwd=ROOT, env=build_env, check=True, stdout=subprocess.DEVNULL)
    lab.binary_hash = digest(lab.binary)
    for wolf in ("a", "b"):
        for _ in range(128):
            lab.reserve(wolf + "_tcp")
            udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            try:
                udp.bind(("127.0.0.1", lab.ports[wolf + "_tcp"]))
            except OSError:
                udp.close()
                lab.release(wolf + "_tcp")
                continue
            lab.ports[wolf + "_quic"] = lab.ports[wolf + "_tcp"]
            lab.reservations[wolf + "_quic"] = udp
            break
        else:
            raise RuntimeError("cannot reserve daemon paired port")
        den = lab.base / wolf
        den.mkdir(mode=0o700)
        write_json(den / "silver.json", {"version": 1, "mode": "open"})
        os.chmod(den / "silver.json", 0o600)
    lab.start_wolves()
    identities = {}
    for wolf in ("a", "b"):
        lab.control(wolf, "pelt.init")
        pelt = json.loads((lab.base / wolf / "pelt.json").read_text())
        identities[wolf] = {k: pelt[k] for k in ("fingerprint", "public_key_b64")}
    lab.stop_wolves()
    def peer(name, identity, address):
        return dict(name=name, trust="Packmate", **identity, address=address)
    for wolf, entries in {
        "a": [peer("wolf-b", identities["b"], f"quic://{lab.address('b_quic')}")],
        "b": [peer("wolf-a", identities["a"], f"quic://127.0.0.1:{tcp_relay.port}")],
    }.items():
        write_json(lab.base / wolf / "pack.json", entries)
    write_json(lab.base / "a" / "target_policy.json", {
        "mode": "deny-by-default", "peers": {
            identities["b"]["fingerprint"]: {
                "targets": [{"address": "127.0.0.1", "port": target.port}]
            }
        }
    })
    write_json(lab.base / "b" / "target_policy.json",
               {"mode": "deny-by-default", "peers": {}})
    lab.start_wolves()
    open_local_fangs(lab, target)
    require(tcp_relay.port == udp_relay.sock.getsockname()[1], "relay port mismatch")


def open_local_fangs(lab, target):
    for transport in ("tcp", "quic"):
        if transport + "_fang" not in lab.ports:
            lab.reserve(transport + "_fang")
            lab.release(transport + "_fang")
        lab.control("b", "fang.open", {
            "peer": "wolf-a", "local": lab.address(transport + "_fang"),
            "remote": f"127.0.0.1:{target.port}", "transport": transport,
        })


def restart_secure_daemons(lab, target):
    # Quinn/server authority can retain recently closed client connections.
    # A fresh process batch keeps the measurement below the existing 64-peer
    # connection cap. Persisted Pelt and Pack identities are unchanged.
    lab.stop_wolves()
    lab.start_wolves()
    open_local_fangs(lab, target)


def run(args):
    os.umask(0o077)
    lab = Lab()
    print(f"Disposable lab: {lab.base}", flush=True)
    target = Target()
    tcp_relay = TcpRelay(("127.0.0.1", 1))
    udp_relay = UdpRelay(tcp_relay.port, ("127.0.0.1", 1))
    try:
        # Daemon ports are reserved in setup; update the already-owned proxies
        # after first reservation, before any measured application connection.
        setup(lab, target, tcp_relay, udp_relay)
        tcp_relay.upstream = ("127.0.0.1", lab.ports["a_tcp"])
        udp_relay.upstream = ("127.0.0.1", lab.ports["a_quic"])
        output = pathlib.Path(args.output)
        output.parent.mkdir(parents=True, exist_ok=True)
        units_dir = output.with_name(output.name + ".units")
        source_head = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        lock_sha = digest(ROOT / "Cargo.lock")
        rows = []
        if output.exists():
            prior = json.loads(output.read_text())
            require(prior["source_head"] == source_head and prior["lock_sha256"] == lock_sha,
                    "cannot resume capture from a different source/lock state")
            require(prior["sample_counts"] == {
                "regular_per_workload_per_transport": args.samples,
                "bulk_per_transport": args.bulk_samples}, "resume sample counts differ")
            require(prior.get("transport_selection", "both") == args.transport,
                    "resume transport selection differs")
            rows = prior["rows"]
        completed = {row["id"] for row in rows}

        def checkpoint():
            value = {
                "schema": 1, "source_head": source_head, "lock_sha256": lock_sha,
                "transport_selection": args.transport,
                "burst_gap_ms": BURST_GAP_MS, "capture_unit": {
                    "tcp": "transparent relay recv calls; TLS record lengths parsed from byte stream",
                    "quic": "transparent UDP relay datagrams",
                },
                "sample_counts": {"regular_per_workload_per_transport": args.samples,
                                  "bulk_per_transport": args.bulk_samples},
                "rows": rows,
            }
            pending = output.with_name(output.name + ".pending")
            write_json(pending, value)
            pending.replace(output)

        for transport in (("tcp", "quic") if args.transport == "both" else ("tcp",)):
            for workload_index, workload in enumerate(WORKLOADS):
                if transport == "quic" and workload_index > 0:
                    restart_secure_daemons(lab, target)
                count = args.samples if workload != "bulk_50mib" else args.bulk_samples
                for sample in range(count):
                    session_id = f"{transport}-{workload}-{sample:02d}"
                    if session_id in completed:
                        continue
                    task = TargetTask(workload)
                    target.tasks.put(task)
                    session = Session(transport, workload, sample)
                    proxy = tcp_relay if transport == "tcp" else udp_relay
                    proxy.active = session
                    try:
                        application_workload(lab.ports[transport + "_fang"], task, session)
                        require(task.accepted.wait(20), "target never accepted")
                        session.target_accept_ns = task.accept_ns
                        if transport == "tcp":
                            require(session.bridge_done.wait(20), "TCP relay did not close")
                        else:
                            time.sleep(.15)  # Capture close/maintenance tail; not workload synchronization.
                        require(task.error is None, f"target error: {task.error}")
                        row = features(session)
                        require(row["count_c2s"] > 0 and row["count_s2c"] > 0,
                                "empty bidirectional transport observation")
                        row["unit_metadata_sha256"] = retain_disposable_unit_metadata(
                            session, units_dir)
                        rows.append(row)
                        checkpoint()
                    finally:
                        proxy.active = None
                    print(f"PASS {transport} {workload} {sample + 1}/{count}", flush=True)
        checkpoint()
        print(f"COMPLETE {len(rows)} independent secure connections", flush=True)
    finally:
        tcp_relay.close()
        udp_relay.close()
        target.close()
        lab.cleanup()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=30)
    parser.add_argument("--bulk-samples", type=int, default=5)
    parser.add_argument("--transport", choices=("both", "tcp"), default="both")
    parser.add_argument("--output", default="tests/stage13_traffic_morphology/captures.json")
    run(parser.parse_args())
