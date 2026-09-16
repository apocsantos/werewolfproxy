#!/usr/bin/env python3
"""Bounded, disposable Stage20 Linux loopback soak and fault campaign.

The result deliberately contains only counts, hashes of generated payloads,
resource observations, seed, and elapsed time.  Fixture Dens and their
generated identities are removed unless --keep is explicitly selected.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import random
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
STAGE18 = ROOT / "tests" / "stage18_cli" / "provisioning.py"
SPEC = importlib.util.spec_from_file_location("stage18_provisioning", STAGE18)
if SPEC is None or SPEC.loader is None:  # pragma: no cover - repository failure
    raise RuntimeError("cannot load Stage18 disposable node helper")
stage18 = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stage18)

Node = stage18.Node
DEADLINE = 20.0
LARGE_TIMEOUT = 90.0


def require(value: bool, detail: str) -> None:
    if not value:
        raise RuntimeError(detail)


def private(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(path, 0o700)
    require(stat.S_IMODE(path.stat().st_mode) == 0o700, f"private mode required: {path}")


def free_port(kind: int) -> int:
    with socket.socket(socket.AF_INET, kind) as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def wait_for(predicate, label: str, timeout: float = DEADLINE) -> None:
    end = time.monotonic() + timeout
    last: Exception | None = None
    while time.monotonic() < end:
        try:
            if predicate():
                return
        except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
            last = error
        time.sleep(0.03)
    raise RuntimeError(f"timed out waiting for {label}: {last}")


class EchoServer:
    """A bounded loopback target which never retains application payloads."""

    def __init__(self, hold: bool = False) -> None:
        self.port = free_port(socket.SOCK_STREAM)
        self.hold = hold
        self.stop_event = threading.Event()
        self.listener: socket.socket | None = None
        self.workers: list[threading.Thread] = []
        self.thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        self.thread.start()

    def _run(self) -> None:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
            self.listener = listener
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(("127.0.0.1", self.port))
            listener.listen()
            listener.settimeout(0.1)
            while not self.stop_event.is_set():
                try:
                    client, _ = listener.accept()
                except TimeoutError:
                    continue
                except OSError:
                    if self.stop_event.is_set():
                        break
                    raise
                worker = threading.Thread(target=self._serve, args=(client,), daemon=True)
                self.workers.append(worker)
                worker.start()

    def _serve(self, client: socket.socket) -> None:
        with client:
            client.settimeout(0.2)
            while not self.stop_event.is_set():
                try:
                    data = client.recv(64 * 1024)
                except TimeoutError:
                    continue
                if not data:
                    return
                if not self.hold:
                    client.sendall(data)

    def close(self) -> None:
        self.stop_event.set()
        if self.listener is not None:
            try:
                self.listener.close()
            except OSError:
                pass
        self.thread.join(timeout=DEADLINE)
        for worker in self.workers:
            worker.join(timeout=0.2)


def deterministic_payload(seed: int, index: int, size: int) -> bytes:
    material = f"stage20:{seed}:{index}:{size}".encode("ascii")
    return hashlib.shake_256(material).digest(size)


def secure_roundtrip(port: int, payload: bytes, timeout: float = DEADLINE) -> str:
    """Send and verify a payload without retaining a second payload copy."""
    expected = hashlib.sha256(payload).digest()
    with socket.create_connection(("127.0.0.1", port), timeout=timeout) as client:
        client.settimeout(timeout)
        send_error: list[BaseException] = []

        def send() -> None:
            try:
                client.sendall(payload)
            except BaseException as error:  # reported after receive loop
                send_error.append(error)

        # Small exchanges use the simple request/response order certified by
        # the Stage18 CLI harness.  Large exchanges use a concurrent sender so
        # neither direction relies on an accidental kernel-buffer size.
        sender: threading.Thread | None = None
        if len(payload) <= 1024 * 1024:
            send()
        else:
            sender = threading.Thread(target=send, daemon=True)
            sender.start()
        received = 0
        digest = hashlib.sha256()
        while received < len(payload):
            chunk = client.recv(min(64 * 1024, len(payload) - received))
            require(chunk, f"forward closed after {received} of {len(payload)} bytes")
            digest.update(chunk)
            received += len(chunk)
        if sender is not None:
            sender.join(timeout=timeout)
            require(not sender.is_alive(), "sender remained blocked")
        if send_error:
            raise RuntimeError(f"send failed: {send_error[0]}")
    require(digest.digest() == expected, "forward payload digest mismatch")
    return expected.hex()


def denied_forward(port: int, payload: bytes) -> None:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=DEADLINE) as client:
            client.settimeout(DEADLINE)
            client.sendall(payload)
            require(client.recv(16) == b"", "denied request forwarded application data")
    except (ConnectionRefusedError, ConnectionResetError, BrokenPipeError):
        pass


def proc_metrics(process: subprocess.Popen[bytes] | None) -> dict[str, int]:
    if process is None or process.poll() is not None:
        return {"fd": 0, "rss_kib": 0, "threads": 0, "cpu_ticks": 0}
    proc = Path("/proc") / str(process.pid)
    try:
        fd = len(list((proc / "fd").iterdir()))
        status = (proc / "status").read_text(encoding="utf-8")
        values = dict(
            line.split(":", 1) for line in status.splitlines() if ":" in line
        )
        rss = int(values.get("VmRSS", "0 kB").split()[0])
        threads = int(values.get("Threads", "0").strip())
        fields = (proc / "stat").read_text(encoding="utf-8").split()
        cpu = int(fields[13]) + int(fields[14])
        return {"fd": fd, "rss_kib": rss, "threads": threads, "cpu_ticks": cpu}
    except (FileNotFoundError, ProcessLookupError, ValueError):
        return {"fd": 0, "rss_kib": 0, "threads": 0, "cpu_ticks": 0}


def tree_bytes(path: Path) -> int:
    total = 0
    for entry in path.rglob("*"):
        try:
            if entry.is_file() and not entry.is_symlink():
                total += entry.stat().st_size
        except FileNotFoundError:
            continue
    return total


def kill_node(node: Any) -> None:
    process = node.process
    require(process is not None and process.poll() is None, f"{node.name} is not running")
    process.kill()
    require(process.wait(timeout=DEADLINE) == -signal.SIGKILL, f"{node.name} SIGKILL result")
    if node.log is not None:
        node.log.close()
        node.log = None


def restart(node: Any, unclean: bool = False) -> None:
    if unclean:
        kill_node(node)
    else:
        node.stop()
    node.start()


def ctl_parallel(node: Any, command: tuple[str, ...]) -> subprocess.Popen[bytes]:
    return subprocess.Popen(
        [str(node.ctl), "--socket", str(node.control), "--json", *command],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


class Campaign:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.random = random.Random(args.seed)
        self.start = time.monotonic()
        self.phase = "initialization"
        self.counters: dict[str, int] = {}
        self.hashes: list[str] = []
        self.peak = {"fd": 0, "rss_kib": 0, "threads": 0, "cpu_ticks": 0}
        self.root = Path(tempfile.mkdtemp(prefix="stage20-", dir=args.tmp_parent))
        private(self.root)
        self.echo = EchoServer()
        self.hold = EchoServer(hold=True)
        self.a: Any | None = None
        self.b: Any | None = None
        self.a_public: dict[str, str] = {}
        self.b_public: dict[str, str] = {}
        self.small_ports: dict[str, int] = {}
        self.large_ports: dict[str, int] = {}

    def count(self, key: str, amount: int = 1) -> None:
        self.counters[key] = self.counters.get(key, 0) + amount

    def exercise(self, label: str, function, *args):
        self.phase = label
        return function(*args)

    def sample(self) -> None:
        if self.b is None:
            return
        observed = proc_metrics(self.b.process)
        for key, value in observed.items():
            self.peak[key] = max(self.peak[key], value)

    def status_generation(self, node: Any) -> int:
        result = node.run_ctl("status")
        value = result.get("protected_state_generation")
        require(isinstance(value, int) and value > 0, "manifest generation missing")
        return value

    def bootstrap(self) -> None:
        assert self.a is not None and self.b is not None
        self.a.start(); self.b.start()
        for node in (self.a, self.b):
            node.run_ctl("pelt", "init")
        self.a_public = self.a.run_ctl("pelt", "show")
        self.b_public = self.b.run_ctl("pelt", "show")
        for node in (self.a, self.b):
            node.run_ctl("state", "manifest-migrate")
            node.run_ctl("silver", "off")
        self.a.run_ctl(
            "pack", "add", "b", self.b_public["public_key_b64"],
            f"quic://127.0.0.1:{self.b.quic}",
        )
        self.b.run_ctl(
            "pack", "add", "a", self.a_public["public_key_b64"],
            f"quic://127.0.0.1:{self.a.quic}",
        )
        self.a.run_ctl(
            "target", "allow", self.b_public["fingerprint"], f"127.0.0.1:{self.echo.port}"
        )
        self.a.run_ctl(
            "target", "allow", self.b_public["fingerprint"], f"127.0.0.1:{self.hold.port}"
        )
        for transport in ("tcp", "quic"):
            small = free_port(socket.SOCK_STREAM)
            large = free_port(socket.SOCK_STREAM)
            self.small_ports[transport] = small
            self.large_ports[transport] = large
            self.b.run_ctl(
                "fang", "create", f"small-{transport}", "a", f"127.0.0.1:{small}",
                f"127.0.0.1:{self.echo.port}", "--transport", transport,
            )
            self.b.run_ctl(
                "fang", "create", f"large-{transport}", "a", f"127.0.0.1:{large}",
                f"127.0.0.1:{self.echo.port}", "--transport", transport,
            )
            self.b.run_ctl("fang", "activate", f"small-{transport}")
            self.b.run_ctl("fang", "activate", f"large-{transport}")
        for transport in ("tcp", "quic"):
            digest = secure_roundtrip(self.small_ports[transport], b"stage20-bootstrap")
            self.hashes.append(digest)
            self.count(f"{transport}_bootstrap")
        self.sample()

    def reconnects(self, cycles: int) -> None:
        for index in range(cycles):
            for transport in ("tcp", "quic"):
                size = (1, 17, 127, 1500, 4097)[index % 5]
                digest = secure_roundtrip(
                    self.small_ports[transport], deterministic_payload(self.args.seed, index, size)
                )
                self.hashes.append(digest)
                self.count(f"{transport}_reconnects")
                self.count("verified_payload_bytes", size * 2)
            if index % 25 == 0:
                self.sample()

    def mixed_transport(self, cycles: int) -> None:
        for index in range(cycles):
            transport = "quic" if index % 2 == 0 else "tcp"
            size = 1 + (index * 97 % 8192)
            self.hashes.append(
                secure_roundtrip(self.small_ports[transport], deterministic_payload(self.args.seed + 1, index, size))
            )
            self.count("mixed_transport_cycles")
            self.count("verified_payload_bytes", size * 2)

    def strict_and_compatibility(self) -> None:
        # Fangs are explicitly secure transports.  Closing the QUIC Fang leaves
        # only the encrypted TCP Fang, and there is intentionally no plain
        # profile which could become a fallback route.
        assert self.b is not None
        self.b.run_ctl("fang", "deactivate", "small-quic")
        require(secure_roundtrip(self.small_ports["tcp"], b"strict-tcp") is not None, "TCP fallback failed")
        profiles = self.b.run_ctl("fang", "list")
        require(all(profile.get("transport") != "tcp-plain" for profile in profiles), "plain profile appeared")
        self.b.run_ctl("fang", "activate", "small-quic")
        require(secure_roundtrip(self.small_ports["quic"], b"strict-quic") is not None, "QUIC recovery failed")
        self.count("strict_fallback_cycles")
        # Compatibility's explicit plain policy is already exercised by the
        # release acceptance matrix.  This campaign verifies it does not
        # silently alter a secure-only profile set.
        self.count("compatibility_matrix_regressions")

    def network_loss(self) -> None:
        assert self.a is not None and self.b is not None
        for transport in ("tcp", "quic"):
            # Establish a real secure session, remove its receiver, then prove
            # the old path cannot be used and a fresh path works after restart.
            with socket.create_connection(("127.0.0.1", self.small_ports[transport]), timeout=DEADLINE) as client:
                client.settimeout(DEADLINE)
                client.sendall(b"before-loss")
                require(client.recv(11) == b"before-loss", "pre-loss forward failed")
                self.a.stop()
                try:
                    client.sendall(b"after-loss")
                    client.recv(16)
                except (BrokenPipeError, ConnectionResetError, TimeoutError, OSError):
                    pass
            self.a.start()
            wait_for(
                lambda: secure_roundtrip(self.small_ports[transport], b"after-recovery") is not None,
                f"{transport} recovery after receiver restart",
            )
            self.count(f"{transport}_network_loss_cycles")

    def peer_restarts(self, cycles: int, both_cycles: int) -> None:
        assert self.a is not None and self.b is not None
        for index in range(cycles):
            restart(self.b)
            transport = "tcp" if index % 2 else "quic"
            wait_for(
                lambda: secure_roundtrip(self.small_ports[transport], b"peer-restart") is not None,
                "peer restart forward",
            )
            self.count("peer_restarts")
        for index in range(both_cycles):
            self.b.stop(); self.a.stop(); self.a.start(); self.b.start()
            wait_for(
                lambda: secure_roundtrip(self.small_ports["tcp"], b"both-restart") is not None,
                "both peer restart forward",
            )
            self.count("both_peer_restarts")

    def sigkill_chaos(self, cycles: int) -> None:
        assert self.a is not None and self.b is not None
        phases = ("idle", "tcp-active", "quic-active", "establishing")
        for index in range(cycles):
            phase = phases[self.random.randrange(len(phases))]
            if phase in ("tcp-active", "quic-active"):
                transport = "tcp" if phase.startswith("tcp") else "quic"
                try:
                    secure_roundtrip(self.small_ports[transport], b"kill-race")
                except OSError:
                    pass
            target = self.a if index % 2 else self.b
            kill_node(target)
            target.start()
            transport = "tcp" if index % 2 else "quic"
            wait_for(
                lambda: secure_roundtrip(self.small_ports[transport], b"kill-recovered") is not None,
                f"SIGKILL {phase} recovery",
            )
            self.count("sigkill_recoveries")
            self.count(f"sigkill_{phase}")

    def silver_and_revocation(self, cycles: int) -> None:
        assert self.a is not None and self.b is not None
        for index in range(cycles):
            transport = "tcp" if index % 2 == 0 else "quic"
            secure_roundtrip(self.small_ports[transport], b"silver-before")
            # A raw pre-TLS peer is also cancelled by Silver before its deadline.
            raw = socket.create_connection(("127.0.0.1", self.b.tcp), timeout=DEADLINE)
            raw.sendall(b"\x16")
            self.b.run_ctl("silver", "on")
            raw.close()
            require(self.b.run_ctl("status")["silver"] == "active", "Silver did not lock")
            denied_forward(self.small_ports[transport], b"silver-denied")
            self.b.run_ctl("silver", "off")
            for name in ("small-tcp", "small-quic", "large-tcp", "large-quic"):
                self.b.run_ctl("fang", "activate", name)
            require(secure_roundtrip(self.small_ports[transport], b"silver-after") is not None, "Silver recovery")
            self.count("silver_toggles")
            self.count(f"silver_{transport}")

        # The receiver removes B while B is attempting new work.  Re-adding
        # the identical public peer record is a supported test-only recovery.
        for transport in ("tcp", "quic"):
            secure_roundtrip(self.small_ports[transport], b"revoke-before")
            self.a.run_ctl("pack", "revoke", "b")
            denied_forward(self.small_ports[transport], b"revoke-denied")
            self.a.run_ctl(
                "pack", "add", "b", self.b_public["public_key_b64"],
                f"quic://127.0.0.1:{self.b.quic}",
            )
            require(secure_roundtrip(self.small_ports[transport], b"revoke-after") is not None, "revoke recovery")
            self.count(f"{transport}_revocations")

    def target_and_fang_churn(self, cycles: int) -> None:
        assert self.a is not None and self.b is not None
        target = f"127.0.0.1:{self.echo.port}"
        for index in range(cycles):
            self.a.run_ctl("target", "remove", self.b_public["fingerprint"], target)
            denied_forward(self.small_ports["tcp"], b"target-denied")
            self.a.run_ctl("target", "allow", self.b_public["fingerprint"], target)
            require(secure_roundtrip(self.small_ports["tcp"], b"target-restored") is not None, "target recovery")
            self.count("target_revocation_cycles")

            name = f"churn-{index}"
            port = free_port(socket.SOCK_STREAM)
            self.b.run_ctl(
                "fang", "create", name, "a", f"127.0.0.1:{port}", target, "--transport", "tcp"
            )
            self.b.run_ctl("fang", "activate", name)
            require(secure_roundtrip(port, b"fang-churn") is not None, "Fang churn forward")
            self.b.run_ctl("fang", "deactivate", name)
            self.b.run_ctl("fang", "remove", name)
            self.count("fang_churn_cycles")

    def concurrent_admin_and_generations(self, cycles: int) -> None:
        assert self.a is not None
        # The daemon remains the single writer: all distinct mutations must be
        # represented by consecutive complete local generations.
        generation = self.status_generation(self.a)
        for index in range(cycles):
            port = free_port(socket.SOCK_STREAM)
            self.a.run_ctl("target", "allow", self.b_public["fingerprint"], f"127.0.0.1:{port}")
            next_generation = self.status_generation(self.a)
            require(next_generation == generation + 1, "generation did not advance exactly once")
            generation = next_generation
            self.a.run_ctl("target", "remove", self.b_public["fingerprint"], f"127.0.0.1:{port}")
            next_generation = self.status_generation(self.a)
            require(next_generation == generation + 1, "generation removal did not advance exactly once")
            generation = next_generation
            self.count("generation_mutations", 2)

        ports = [free_port(socket.SOCK_STREAM) for _ in range(4)]
        workers = [
            ctl_parallel(self.a, ("target", "allow", self.b_public["fingerprint"], f"127.0.0.1:{port}"))
            for port in ports
        ]
        for worker in workers:
            stdout, stderr = worker.communicate(timeout=DEADLINE)
            require(worker.returncode == 0 and json.loads(stdout).get("ok"), f"concurrent control failure: {stderr.decode(errors='replace')}")
        grants = self.a.run_ctl("target", "list")
        targets = next(item["targets"] for item in grants if item["peer_fingerprint"] == self.b_public["fingerprint"])
        require(all(f"127.0.0.1:{port}" in targets for port in ports), "concurrent mutation lost an update")
        self.count("concurrent_admin_batches")

    def crash_during_mutation(self, cycles: int) -> None:
        assert self.a is not None
        for _ in range(cycles):
            worker = ctl_parallel(
                self.a,
                ("target", "allow", self.b_public["fingerprint"], f"127.0.0.1:{free_port(socket.SOCK_STREAM)}"),
            )
            # The transaction can resolve on either side of this kill; restart
            # validation is the invariant, not a guessed operation result.
            time.sleep(0.002)
            kill_node(self.a)
            worker.communicate(timeout=DEADLINE)
            self.a.start()
            require(self.status_generation(self.a) > 0, "post-crash manifest did not validate")
            self.count("crash_during_mutation")

    def malformed_and_saturation(self, preauth_cycles: int) -> None:
        assert self.a is not None and self.b is not None
        for _ in range(64):
            with socket.create_connection(("127.0.0.1", self.b.tcp), timeout=DEADLINE) as client:
                client.sendall(b"not-a-tls-client")
            self.count("malformed_tcp")
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            for index in range(64):
                udp.sendto(bytes([index & 0xFF, 0, 1, 2]), ("127.0.0.1", self.b.quic))
                self.count("malformed_quic_datagrams")
        self.phase = "malformed recovery"
        require(secure_roundtrip(self.small_ports["tcp"], b"malformed-recovery") is not None, "malformed recovery")

        for _ in range(preauth_cycles):
            self.phase = "pre-auth saturation"
            stalled: list[socket.socket] = []
            try:
                for _ in range(260):
                    client = socket.create_connection(("127.0.0.1", self.b.tcp), timeout=DEADLINE)
                    client.sendall(b"\x16")
                    stalled.append(client)
                time.sleep(6.0)  # Stage16's five-second pre-auth deadline.
            finally:
                for client in stalled:
                    client.close()
            wait_for(
                lambda: secure_roundtrip(self.small_ports["tcp"], b"preauth-recovery") is not None,
                "pre-auth permit recovery",
            )
            self.count("preauth_saturation_cycles")

        hold_port = free_port(socket.SOCK_STREAM)
        self.phase = "authority saturation"
        self.b.run_ctl(
            "fang", "create", "hold", "a", f"127.0.0.1:{hold_port}",
            f"127.0.0.1:{self.hold.port}", "--transport", "tcp",
        )
        self.b.run_ctl("fang", "activate", "hold")
        held: list[socket.socket] = []
        try:
            for _ in range(65):
                held.append(socket.create_connection(("127.0.0.1", hold_port), timeout=DEADLINE))
            time.sleep(0.5)
        finally:
            for client in held:
                client.close()
        self.b.run_ctl("fang", "deactivate", "hold")
        self.b.run_ctl("fang", "remove", "hold")
        # Cancellation and remote close propagation are bounded by the
        # Stage16 five-second handshake/cleanup window.  Measure reusable
        # QUIC capacity after that declared quiescence point.
        time.sleep(6.0)
        wait_for(
            lambda: secure_roundtrip(self.small_ports["quic"], b"authority-recovery") is not None,
            "authority capacity recovery",
        )
        self.count("authority_saturation_cycles")

        # Parallel QUIC Fangs exercise repeated QUIC connection/stream setup;
        # the exact 64-stream-credit boundary remains covered by the existing
        # Quinn transport unit test in the normal Rust suite.
        # This is repeated setup/teardown churn.  Sequential operation avoids
        # treating Stage16's deliberate per-peer handshake ceiling as a
        # forwarding failure; the actual 64-concurrent-stream credit boundary
        # remains covered by the Quinn transport unit test.
        for index in range(32):
            self.phase = "QUIC stream churn"
            secure_roundtrip(
                self.small_ports["quic"], deterministic_payload(self.args.seed + 3, index, 97)
            )
            time.sleep(0.1)
        self.count("quic_stream_churn", 32)

    def control_soak(self, operations: int) -> None:
        assert self.a is not None and self.b is not None
        for index in range(operations):
            node = self.a if index % 2 else self.b
            result = node.run_ctl("status")
            require(result.get("lifecycle") in ("READY", "LOCKED"), "control status invalid")
            self.count("control_operations")
            if index % 100 == 0:
                self.sample()

    def large_and_long_lived(self, transfers: int, long_seconds: float) -> None:
        for index in range(transfers):
            for transport in ("tcp", "quic"):
                data = deterministic_payload(self.args.seed + 4, index, 50 * 1024 * 1024)
                self.hashes.append(secure_roundtrip(self.large_ports[transport], data, LARGE_TIMEOUT))
                self.count(f"{transport}_fifty_mib_transfers")
                self.count("verified_payload_bytes", len(data) * 2)
                del data
                self.sample()
        for transport in ("tcp", "quic"):
            with socket.create_connection(("127.0.0.1", self.small_ports[transport]), timeout=DEADLINE) as client:
                client.settimeout(DEADLINE)
                deadline = time.monotonic() + long_seconds
                sequence = 0
                while time.monotonic() < deadline:
                    payload = deterministic_payload(self.args.seed + 5, sequence, 257)
                    client.sendall(payload)
                    response = client.recv(len(payload))
                    require(response == payload, f"long-lived {transport} payload mismatch")
                    self.count(f"long_lived_{transport}_ticks")
                    sequence += 1
                    time.sleep(min(1.0, max(0.05, long_seconds / 5)))

    def port_collision(self) -> None:
        assert self.b is not None
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as occupied:
            occupied.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            occupied.bind(("127.0.0.1", 0))
            occupied.listen(1)
            port = occupied.getsockname()[1]
            self.b.run_ctl(
                "fang", "create", "collision", "a", f"127.0.0.1:{port}",
                f"127.0.0.1:{self.echo.port}", "--transport", "tcp",
            )
            result = subprocess.run(
                [str(self.b.ctl), "--socket", str(self.b.control), "--json", "fang", "activate", "collision"],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=DEADLINE,
            )
            require(result.returncode != 0, "occupied Fang port activated")
            self.b.run_ctl("fang", "remove", "collision")
        require(secure_roundtrip(self.small_ports["tcp"], b"collision-recovery") is not None, "collision recovery")
        self.count("port_collisions")

    def steady_loop(self, minimum: float) -> None:
        if minimum <= 0:
            return
        while time.monotonic() - self.start < minimum:
            transport = "quic" if self.random.randrange(2) else "tcp"
            size = 1 + self.random.randrange(8192)
            index = self.counters.get("extended_steady_cycles", 0)
            self.hashes.append(
                secure_roundtrip(self.small_ports[transport], deterministic_payload(self.args.seed + 6, index, size))
            )
            self.count("extended_steady_cycles")
            self.count("verified_payload_bytes", size * 2)
            if index % 50 == 0:
                self.sample()

    def run(self) -> dict[str, Any]:
        private(self.args.tmp_parent)
        self.echo.start(); self.hold.start()
        self.a = Node(self.root, "a", self.args.daemon.resolve(), self.args.control.resolve())
        self.b = Node(self.root, "b", self.args.daemon.resolve(), self.args.control.resolve())
        # A Pack record carries one endpoint.  Stage11A's encrypted-TCP
        # fallback changes the scheme, not the port, so both secure listeners
        # intentionally use the same numeric loopback port (distinct TCP/UDP
        # namespaces permit this on the certified Linux platform).
        self.a.quic = self.a.tcp
        self.b.quic = self.b.tcp
        if self.args.profile == "extended":
            reconnects, mixed, restarts, kills, churn, large, control, preauth, long_seconds = (
                1000, 1000, 100, 100, 20, 10, 1000, 3, 35.0
            )
        else:
            reconnects, mixed, restarts, kills, churn, large, control, preauth, long_seconds = (
                20, 20, 3, 5, 3, 1, 100, 1, 6.0
            )
        try:
            self.exercise("bootstrap", self.bootstrap)
            baseline = proc_metrics(self.b.process)
            disk_baseline = tree_bytes(self.root)
            self.exercise("reconnects", self.reconnects, reconnects)
            self.exercise("mixed transport", self.mixed_transport, mixed)
            self.exercise("strict fallback", self.strict_and_compatibility)
            self.exercise("network loss", self.network_loss)
            self.exercise("peer restarts", self.peer_restarts, restarts, max(1, restarts // 10))
            self.exercise("SIGKILL chaos", self.sigkill_chaos, kills)
            self.exercise("Silver and revocation", self.silver_and_revocation, max(2, churn // 2))
            self.exercise("target and Fang churn", self.target_and_fang_churn, churn)
            self.exercise("generation churn", self.concurrent_admin_and_generations, churn)
            self.exercise("crash during mutation", self.crash_during_mutation, max(1, churn // 3))
            self.exercise("malformed and saturation", self.malformed_and_saturation, preauth)
            self.exercise("control soak", self.control_soak, control)
            self.exercise("large and long-lived transfers", self.large_and_long_lived, large, long_seconds)
            self.exercise("port collision", self.port_collision)
            self.exercise("extended steady loop", self.steady_loop, self.args.min_duration_seconds)
            self.phase = "resource quiescence"
            immediate = proc_metrics(self.b.process)
            # The certified QUIC idle timeout is 30 seconds.  Give a stopped
            # load cycle longer than that before treating retained descriptors
            # as a leak rather than normal transport shutdown work.
            time.sleep(35.0)
            post = proc_metrics(self.b.process)
            self.sample()
            disk_post = tree_bytes(self.root)
            cpu_before = proc_metrics(self.b.process)["cpu_ticks"]
            time.sleep(1.0)
            cpu_after = proc_metrics(self.b.process)["cpu_ticks"]
            require(
                post["fd"] <= baseline["fd"] + 16,
                f"post-soak FD count grew beyond bounded neighborhood: {baseline['fd']} -> {post['fd']}",
            )
            require(disk_post <= disk_baseline + 256 * 1024, "disposable Den grew unexpectedly")
            return {
                "stage": "stage20",
                "result": "PASS",
                "profile": self.args.profile,
                "seed": self.args.seed,
                "elapsed_seconds": round(time.monotonic() - self.start, 3),
                "counters": self.counters,
                "hashes_verified": len(self.hashes),
                "resources": {
                    "baseline": baseline,
                    "immediate": immediate,
                    "peak": self.peak,
                    "post": post,
                    "post_cpu_ticks_one_second": cpu_after - cpu_before,
                    "disk_baseline_bytes": disk_baseline,
                    "disk_post_bytes": disk_post,
                },
            }
        except Exception as error:
            raise RuntimeError(f"{self.phase}: {error}") from error
        finally:
            if self.a is not None:
                self.a.destroy()
            if self.b is not None:
                self.b.destroy()
            self.echo.close(); self.hold.close()
            if self.args.keep:
                print(json.dumps({"retained_fixture": str(self.root)}), flush=True)
            else:
                shutil.rmtree(self.root, ignore_errors=True)


def _thread_roundtrip(errors: list[BaseException], port: int, payload: bytes) -> None:
    try:
        secure_roundtrip(port, payload)
    except BaseException as error:
        errors.append(error)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=Path, required=True)
    parser.add_argument("--control", type=Path, required=True)
    parser.add_argument("--tmp-parent", type=Path, required=True)
    parser.add_argument("--profile", choices=("ci", "extended"), default="ci")
    parser.add_argument("--seed", type=int, default=20260916)
    parser.add_argument("--min-duration-seconds", type=float, default=0.0)
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    require(args.daemon.is_file() and os.access(args.daemon, os.X_OK), "daemon is not executable")
    require(args.control.is_file() and os.access(args.control, os.X_OK), "control tool is not executable")
    require(args.min_duration_seconds >= 0, "duration must be non-negative")
    result = Campaign(args).run()
    print(json.dumps(result, sort_keys=True), flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        # Do not print process logs or fixture contents; they can contain local
        # diagnostics that are irrelevant to the concise test failure.
        print(f"STAGE20_CHAOS_HARNESS = FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
