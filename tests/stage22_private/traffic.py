#!/usr/bin/env python3
"""Installed-artifact loopback route churn, concurrent Fang, and transfer test."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import socket
import stat
import subprocess
import sys
import shutil
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
STAGE20_PATH = ROOT / "tests/stage20_chaos/soak.py"
SPEC = importlib.util.spec_from_file_location("stage22_stage20", STAGE20_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load existing loopback target and process-measurement helpers")
stage20 = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stage20)
Node = stage20.Node
EchoServer = stage20.EchoServer


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def private(path: Path) -> None:
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(path, 0o700)
    require(stat.S_IMODE(path.stat().st_mode) == 0o700, f"temporary parent must be mode 0700: {path}")


def free_port(sock_type: int) -> int:
    with socket.socket(socket.AF_INET, sock_type) as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def recv_exact(client: socket.socket, count: int) -> bytes:
    chunks = bytearray()
    while len(chunks) < count:
        part = client.recv(count - len(chunks))
        require(part, f"route closed after {len(chunks)} of {count} response bytes")
        chunks.extend(part)
    return bytes(chunks)


def route_churn(port: int, transport: str, count: int) -> None:
    for index in range(count):
        token = f"s22:{transport}:{index:05}".encode()
        with socket.create_connection(("127.0.0.1", port), timeout=15) as client:
            client.settimeout(15)
            client.sendall(token)
            require(recv_exact(client, len(token)) == token,
                    f"{transport} route {index} returned different bytes")
        if (index + 1) % 1000 == 0:
            print(f"PROGRESS {transport} routes={index + 1}", flush=True)


def chunk(index: int, size: int) -> bytes:
    return hashlib.shake_256(f"stage22-private-transfer:{index}".encode()).digest(size)


def transfer(port: int, transport: str, total: int) -> str:
    chunk_size = 64 * 1024
    sent_hash = hashlib.sha256()
    received_hash = hashlib.sha256()
    with socket.create_connection(("127.0.0.1", port), timeout=60) as client:
        client.settimeout(60)
        offset = 0
        index = 0
        while offset < total:
            payload = chunk(index, min(chunk_size, total - offset))
            sent_hash.update(payload)
            client.sendall(payload)
            received = recv_exact(client, len(payload))
            received_hash.update(received)
            offset += len(payload)
            index += 1
            if offset % (64 * 1024 * 1024) == 0 or offset == total:
                print(f"PROGRESS {transport} transfer_mib={offset // (1024 * 1024)}", flush=True)
    require(sent_hash.digest() == received_hash.digest(), f"{transport} transfer SHA-256 mismatch")
    return sent_hash.hexdigest()


def long_session(port: int, transport: str, duration: float) -> dict[str, Any]:
    start = time.monotonic()
    count = 0
    with socket.create_connection(("127.0.0.1", port), timeout=20) as client:
        client.settimeout(20)
        while time.monotonic() - start < duration:
            token = hashlib.sha256(f"stage22-session:{transport}:{count}".encode()).digest()
            client.sendall(token)
            require(recv_exact(client, len(token)) == token,
                    f"{transport} long-lived application exchange {count} failed")
            count += 1
            if count % 240 == 0:
                print(f"PROGRESS {transport} long_session_seconds={int(time.monotonic() - start)} exchanges={count}", flush=True)
            time.sleep(0.25)
    return {"transport": transport, "duration_seconds": round(time.monotonic() - start, 2),
            "application_exchanges": count}


def offline_json(control: Path, home: Path, *command: str,
                 expected: int | None = 0) -> dict[str, Any]:
    completed = subprocess.run(
        [str(control), "--home", str(home), "--json", *command],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20,
    )
    require((completed.returncode == expected) if expected is not None else completed.returncode != 0,
            f"offline {command[0]} exited {completed.returncode}, expected {expected}")
    return json.loads(completed.stdout)


def restored_den_identity(daemon: Path, control: Path, home: Path,
                          runtime: Path) -> tuple[str, dict[str, Any]]:
    """Start a restored Den and verify its public identity through its daemon."""
    private(runtime)
    socket_path = runtime / "control.sock"
    log_path = runtime / "daemon.log"
    with log_path.open("wb") as log:
        process = subprocess.Popen(
            [str(daemon), "--home", str(home), "--socket", str(socket_path),
             "--listen", f"127.0.0.1:{free_port(socket.SOCK_STREAM)}",
             "--quic-listen", f"127.0.0.1:{free_port(socket.SOCK_DGRAM)}"],
            stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
        )
        try:
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline and not socket_path.is_socket():
                require(process.poll() is None, "restored-Den daemon exited before readiness")
                time.sleep(0.05)
            require(socket_path.is_socket(), "restored-Den daemon did not become ready")
            pelt = subprocess.run(
                [str(control), "--socket", str(socket_path), "--json", "pelt", "show"],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20,
            )
            require(pelt.returncode == 0, "restored-Den Pelt identity query failed")
            doctor = subprocess.run(
                [str(control), "--home", str(home), "--socket", str(socket_path),
                 "--json", "doctor"],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20,
            )
            require(doctor.returncode == 0, "restored-Den doctor failed")
            doctor_json = json.loads(doctor.stdout)
            require(doctor_json.get("ok") is True, "restored-Den doctor reported failure")
            fingerprint = json.loads(pelt.stdout)["result"]["fingerprint"]
            return fingerprint, doctor_json
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)


def check_private_tree(root: Path) -> None:
    root_info = root.stat()
    require(stat.S_IMODE(root_info.st_mode) == 0o700, "backup root mode is not 0700")
    require(root_info.st_uid == os.geteuid() and root_info.st_gid == os.getegid(),
            "backup root owner/group mismatch")
    for path in root.rglob("*"):
        info = path.lstat()
        require(not stat.S_ISLNK(info.st_mode), "backup contains a symlink")
        require(info.st_uid == os.geteuid() and info.st_gid == os.getegid(),
                "backup component owner/group mismatch")
        if stat.S_ISDIR(info.st_mode):
            require(stat.S_IMODE(info.st_mode) == 0o700, "backup directory mode is not 0700")
        elif stat.S_ISREG(info.st_mode):
            require(stat.S_IMODE(info.st_mode) == 0o600, "backup file mode is not 0600")


def audit_test_logs(root: Path) -> dict[str, int]:
    """Reject obvious secret encodings and known application probes in daemon logs."""
    forbidden = (
        b"-----begin private key-----", b"-----begin rsa private key-----",
        b"pkcs#8", b"pkcs8", b"traffic secret", b"session key",
        b"exporter bytes", b"keylog", b"mapping-probe",
        b"target-before-outage", b"unattended-before",
    )
    logs = list(root.glob("*.log"))
    require(len(logs) >= 2, "expected daemon log files were not created")
    total = 0
    for path in logs:
        content = path.read_bytes().lower()
        total += len(content)
        for marker in forbidden:
            require(marker not in content,
                    f"daemon log contains a forbidden secret/payload marker: {path.name}")
    return {"files": len(logs), "bytes": total}


class MarkerTarget:
    """Target that returns a private test marker to detect cross-Fang routing."""
    def __init__(self, marker: bytes) -> None:
        self.marker = marker
        self.port = free_port(socket.SOCK_STREAM)
        self.stop_event = threading.Event()
        self.listener: socket.socket | None = None
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
                        return
                    raise
                threading.Thread(target=self._serve, args=(client,), daemon=True).start()

    def _serve(self, client: socket.socket) -> None:
        with client:
            try:
                client.settimeout(5)
                if client.recv(256):
                    client.sendall(self.marker)
            except OSError:
                pass

    def close(self) -> None:
        self.stop_event.set()
        if self.listener is not None:
            self.listener.close()
        self.thread.join(timeout=2)


class RestartableEcho:
    """Loopback target that can be taken down and returned on the same port."""
    def __init__(self) -> None:
        self.port = free_port(socket.SOCK_STREAM)
        self.stop_event = threading.Event()
        self.listener: socket.socket | None = None
        self.thread: threading.Thread | None = None

    def start(self) -> None:
        require(self.thread is None or not self.thread.is_alive(), "echo target is already running")
        self.stop_event = threading.Event()
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(("127.0.0.1", self.port))
        listener.listen()
        listener.settimeout(0.1)
        self.listener = listener
        self.thread = threading.Thread(target=self._run, args=(listener,), daemon=True)
        self.thread.start()

    def _run(self, listener: socket.socket) -> None:
        while not self.stop_event.is_set():
            try:
                client, _ = listener.accept()
            except TimeoutError:
                continue
            except OSError:
                if self.stop_event.is_set():
                    return
                raise
            threading.Thread(target=self._serve, args=(client,), daemon=True).start()

    def _serve(self, client: socket.socket) -> None:
        with client:
            client.settimeout(0.2)
            while not self.stop_event.is_set():
                try:
                    data = client.recv(64 * 1024)
                except TimeoutError:
                    continue
                except OSError:
                    return
                if not data:
                    return
                try:
                    client.sendall(data)
                except OSError:
                    return

    def stop(self) -> None:
        self.stop_event.set()
        if self.listener is not None:
            self.listener.close()
        if self.thread is not None:
            self.thread.join(timeout=2)
            require(not self.thread.is_alive(), "echo target failed to stop")
        self.listener = None


def require_unavailable(port: int, transport: str) -> float:
    start = time.monotonic()
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=5) as client:
            client.settimeout(5)
            client.sendall(b"target-outage-probe")
            try:
                response = client.recv(64)
                require(response == b"", f"{transport} forwarded bytes while its target was unavailable")
            except (TimeoutError, socket.timeout):
                pass
    except OSError:
        pass
    elapsed = time.monotonic() - start
    require(elapsed <= 10, f"{transport} unavailable-target failure was not bounded")
    return elapsed


def wait_for_target(port: int, transport: str) -> float:
    start = time.monotonic()
    deadline = start + 30
    attempt = 0
    while time.monotonic() < deadline:
        payload = f"target-recovered:{transport}:{attempt}".encode()
        try:
            stage20.secure_roundtrip(port, payload, timeout=3)
            return time.monotonic() - start
        except (OSError, AssertionError, RuntimeError):
            time.sleep(0.25)
            attempt += 1
    raise RuntimeError(f"{transport} did not recover after target return")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=Path, required=True)
    parser.add_argument("--control", type=Path, required=True)
    parser.add_argument("--tmp-parent", type=Path, required=True)
    parser.add_argument("--tcp-routes", type=int, default=10000)
    parser.add_argument("--quic-routes", type=int, default=10000)
    parser.add_argument("--transfer-mib", type=int, default=1024)
    parser.add_argument("--long-seconds", type=int, default=600)
    parser.add_argument("--idle-seconds", type=int, default=300)
    args = parser.parse_args()
    require(args.tcp_routes >= 0 and args.quic_routes >= 0 and args.transfer_mib >= 0,
            "route and transfer counts must be nonnegative")
    require(args.long_seconds >= 0 and args.idle_seconds >= 0,
            "session and idle durations must be nonnegative")
    private(args.tmp_parent)

    root = Path(tempfile.mkdtemp(prefix="stage22-traffic-", dir=args.tmp_parent))
    os.chmod(root, 0o700)
    echo = EchoServer(); echo.start()
    a = Node(root, "a", args.daemon.resolve(), args.control.resolve())
    b = Node(root, "b", args.daemon.resolve(), args.control.resolve())
    marker_targets = [MarkerTarget(b"target-one"), MarkerTarget(b"target-two")]
    outage_targets: list[RestartableEcho] = []
    tcp_profile = "stage22-tcp"
    quic_profile = "stage22-quic"
    active_profiles: list[str] = []
    result: dict[str, Any] = {"result": "PASS", "topology": "two local independent loopback nodes",
                              "tcp_routes": 0, "quic_routes": 0, "transfers": {},
                              "long_sessions": [], "concurrent_fangs": "NOT RUN",
                              "network_recovery": {}, "resource_samples": {}}
    try:
        a.start(); b.start()
        result["resource_samples"]["baseline"] = stage20.proc_metrics(b.process)
        a_public, b_public = stage20.stage18.exchange(a, b)
        require(a_public["fingerprint"] != b_public["fingerprint"] and
                a_public["public_key_b64"] != b_public["public_key_b64"],
                "the two operational nodes share a Pelt identity")
        result["identity"] = "PASS: independent generated Pelt fingerprints/public keys"
        a.run_ctl("target", "allow", b_public["fingerprint"], f"127.0.0.1:{echo.port}")
        for target in marker_targets:
            target.start()
            a.run_ctl("target", "allow", b_public["fingerprint"], f"127.0.0.1:{target.port}")

        for transport, profile, route_count, endpoint in (
            ("tcp", tcp_profile, args.tcp_routes, f"tcp://127.0.0.1:{a.tcp}"),
            ("quic", quic_profile, args.quic_routes, f"quic://127.0.0.1:{a.quic}"),
        ):
            b.run_ctl("pack", "set-address", "a", endpoint)
            local_port = free_port(socket.SOCK_STREAM)
            b.run_ctl("fang", "create", profile, "a", f"127.0.0.1:{local_port}",
                      f"127.0.0.1:{echo.port}", "--transport", transport)
            b.run_ctl("fang", "activate", profile)
            active_profiles.append(profile)
            route_churn(local_port, transport, route_count)
            result[f"{transport}_routes"] = route_count
            if args.transfer_mib:
                total = args.transfer_mib * 1024 * 1024
                result["transfers"][transport] = {
                    "bytes": total, "sha256": transfer(local_port, transport, total),
                }
            if args.long_seconds:
                before = stage20.proc_metrics(b.process)
                result["long_sessions"].append(long_session(local_port, transport, args.long_seconds))
                result["resource_samples"][f"{transport}_before_idle"] = before
                result["resource_samples"][f"{transport}_after_session"] = stage20.proc_metrics(b.process)
            b.run_ctl("fang", "deactivate", profile)
            b.run_ctl("fang", "remove", profile)
            active_profiles.remove(profile)

            outage_target = RestartableEcho()
            outage_targets.append(outage_target)
            a.run_ctl("target", "allow", b_public["fingerprint"], f"127.0.0.1:{outage_target.port}")
            b.run_ctl("pack", "set-address", "a", endpoint)
            outage_target.start()
            outage_profile = f"stage22-outage-{transport}"
            outage_port = free_port(socket.SOCK_STREAM)
            b.run_ctl("fang", "create", outage_profile, "a", f"127.0.0.1:{outage_port}",
                      f"127.0.0.1:{outage_target.port}", "--transport", transport)
            b.run_ctl("fang", "activate", outage_profile)
            active_profiles.append(outage_profile)
            require(stage20.secure_roundtrip(outage_port, b"target-before-outage") is not None,
                    f"{transport} target pre-outage route failed")
            outage_target.stop()
            unavailable_seconds = require_unavailable(outage_port, transport)
            outage_target.start()
            recovery_seconds = wait_for_target(outage_port, transport)
            outage_target.stop()
            require_unavailable(outage_port, transport)
            outage_target.start()
            final_recovery_seconds = wait_for_target(outage_port, transport)
            result["network_recovery"][transport] = {
                "target_unavailable_bounded_seconds": round(unavailable_seconds, 3),
                "target_return_recovery_seconds": round(recovery_seconds, 3),
                "second_return_recovery_seconds": round(final_recovery_seconds, 3),
                "daemon_restart_required": False,
                "scope": "loopback target process outage; no interface/VLAN/WAN impairment",
            }
            outage_target.stop()
            b.run_ctl("fang", "deactivate", outage_profile)
            b.run_ctl("fang", "remove", outage_profile)
            active_profiles.remove(outage_profile)

        # Two profiles on one node target different permitted applications.
        # Distinct markers demonstrate target mapping and isolation.
        b.run_ctl("pack", "set-address", "a", f"tcp://127.0.0.1:{a.tcp}")
        marker_ports: list[int] = []
        for index, target in enumerate(marker_targets):
            port = free_port(socket.SOCK_STREAM)
            profile = f"stage22-map-{index}"
            b.run_ctl("fang", "create", profile, "a", f"127.0.0.1:{port}",
                      f"127.0.0.1:{target.port}", "--transport", "tcp")
            b.run_ctl("fang", "activate", profile)
            active_profiles.append(profile)
            marker_ports.append(port)
        for port, expected in zip(marker_ports, (b"target-one", b"target-two")):
            with socket.create_connection(("127.0.0.1", port), timeout=15) as client:
                client.settimeout(15)
                client.sendall(b"mapping-probe")
                require(recv_exact(client, len(expected)) == expected,
                        "concurrent Fang returned the other target's marker")
        result["concurrent_fangs"] = "PASS: two concurrent profiles returned their own target markers"
        for index in range(len(marker_targets)):
            profile = f"stage22-map-{index}"
            b.run_ctl("fang", "deactivate", profile)
            b.run_ctl("fang", "remove", profile)
            active_profiles.remove(profile)

        # Peer unavailable, restart the local daemon, restore the peer while
        # its target is still down, then return the same target address. The
        # saved Fang intent and provisioned identities must recover unaided.
        sequence_target = RestartableEcho()
        outage_targets.append(sequence_target)
        sequence_target.start()
        a.run_ctl("target", "allow", b_public["fingerprint"],
                  f"127.0.0.1:{sequence_target.port}")
        b.run_ctl("pack", "set-address", "a", f"tcp://127.0.0.1:{a.tcp}")
        sequence_port = free_port(socket.SOCK_STREAM)
        sequence_profile = "stage22-unattended"
        b.run_ctl("fang", "create", sequence_profile, "a", f"127.0.0.1:{sequence_port}",
                  f"127.0.0.1:{sequence_target.port}", "--transport", "tcp")
        b.run_ctl("fang", "activate", sequence_profile)
        active_profiles.append(sequence_profile)
        require(stage20.secure_roundtrip(sequence_port, b"unattended-before") is not None,
                "unattended recovery setup route failed")
        sequence_target.stop()
        a.stop()
        b.stop(); b.start()
        a.start()
        require(b.run_ctl("status")["silver"] != "active",
                "local daemon restart unexpectedly left Silver open/locked mismatch")
        unavailable_seconds = require_unavailable(sequence_port, "tcp")
        sequence_target.start()
        recovery_seconds = wait_for_target(sequence_port, "tcp")
        require(a.run_ctl("pelt", "show")["fingerprint"] == a_public["fingerprint"] and
                b.run_ctl("pelt", "show")["fingerprint"] == b_public["fingerprint"],
                "unattended recovery changed a node identity")
        result["unattended_recovery"] = {
            "sequence": "peer stopped; local daemon restarted; peer returned; target returned",
            "target_outage_bounded_seconds": round(unavailable_seconds, 3),
            "target_return_recovery_seconds": round(recovery_seconds, 3),
            "manual_reprovisioning_required": False,
            "fingerprints_preserved": True,
        }
        sequence_target.stop()
        b.run_ctl("fang", "deactivate", sequence_profile)
        b.run_ctl("fang", "remove", sequence_profile)
        active_profiles.remove(sequence_profile)

        print(f"PHASE idle_start seconds={args.idle_seconds}", flush=True)
        idle_before = stage20.proc_metrics(b.process)
        idle_den_before = stage20.tree_bytes(a.den) + stage20.tree_bytes(b.den)
        idle_logs_before = stage20.fixture_log_bytes(a, b)
        time.sleep(args.idle_seconds)
        idle_after = stage20.proc_metrics(b.process)
        idle_den_after = stage20.tree_bytes(a.den) + stage20.tree_bytes(b.den)
        idle_logs_after = stage20.fixture_log_bytes(a, b)
        idle_cpu_ticks = idle_after["cpu_ticks"] - idle_before["cpu_ticks"]
        require(idle_after["fd"] <= idle_before["fd"] + 4,
                f"idle FD count grew: {idle_before['fd']} -> {idle_after['fd']}")
        require(idle_after["rss_kib"] <= idle_before["rss_kib"] + 32 * 1024,
                f"idle RSS grew by more than 32 MiB: {idle_before['rss_kib']} -> {idle_after['rss_kib']} KiB")
        require(idle_cpu_ticks <= args.idle_seconds * 2 + 25,
                f"idle CPU use exceeded 2% average allowance: {idle_cpu_ticks} ticks in {args.idle_seconds}s")
        require(idle_logs_after <= idle_logs_before + 64 * 1024,
                f"idle log growth exceeded 64 KiB: {idle_logs_before} -> {idle_logs_after}")
        require(idle_den_after == idle_den_before,
                f"idle Den size changed: {idle_den_before} -> {idle_den_after}")
        print("PASS idle resource bounds", flush=True)
        result["resource_samples"]["idle_before"] = idle_before
        result["resource_samples"]["idle_after"] = idle_after
        result["resource_samples"]["idle_duration_seconds"] = args.idle_seconds
        result["resource_samples"]["idle_cpu_ticks"] = idle_cpu_ticks
        result["resource_samples"]["idle_den_bytes_before"] = idle_den_before
        result["resource_samples"]["idle_den_bytes_after"] = idle_den_after
        result["resource_samples"]["idle_log_bytes_before"] = idle_logs_before
        result["resource_samples"]["idle_log_bytes_after"] = idle_logs_after
        result["resource_samples"]["final_before_shutdown"] = stage20.proc_metrics(b.process)
        result["den_bytes"] = stage20.tree_bytes(a.den) + stage20.tree_bytes(b.den)
        result["log_bytes"] = stage20.fixture_log_bytes(a, b)
        a.stop(); b.stop()
        result["log_audit"] = audit_test_logs(root)
        # A private complete-Den copy is tested only within disposable local
        # state. Pelt outputs are retained in memory solely for fingerprint
        # equality and are never printed or written to the report.
        backup = root / "node-a-backup"
        restored = root / "node-a-restored"
        shutil.copytree(a.den, backup, copy_function=shutil.copy2)
        os.chmod(backup, 0o700)
        check_private_tree(backup)
        shutil.copytree(backup, restored, copy_function=shutil.copy2)
        os.chmod(restored, 0o700)
        check_private_tree(restored)
        after, doctor = restored_den_identity(args.daemon.resolve(), args.control.resolve(),
                                               restored, root / "restored-runtime")
        require(a_public["fingerprint"] == after and doctor.get("ok") is True,
                "restored Den fingerprint or doctor check failed")
        result["backup_restore"] = "PASS: complete stopped-Den copy has private modes; fingerprint and doctor match"

        corrupted = root / "node-a-corrupt-copy"
        shutil.copytree(backup, corrupted, copy_function=shutil.copy2)
        os.chmod(corrupted, 0o700)
        selector = corrupted / "security_state_current.json"
        require(selector.is_file(), "expected protected-state selector absent from backup")
        selector.write_bytes(b"{")
        os.chmod(selector, 0o600)
        corrupted_doctor = offline_json(args.control, corrupted, "doctor", expected=None)
        require(corrupted_doctor.get("ok") is False,
                "doctor accepted deliberately corrupted selector")
        result["corruption"] = "PASS: disposable damaged selector fails doctor closed"
        print(json.dumps(result, sort_keys=True), flush=True)
        print("STAGE22_PRIVATE_TRAFFIC = PASS", flush=True)
        return 0
    finally:
        for profile in active_profiles:
            try:
                b.run_ctl("fang", "deactivate", profile)
            except Exception:
                pass
        a.destroy(); b.destroy(); echo.close()
        for target in marker_targets:
            target.close()
        for target in outage_targets:
            target.stop()
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"STAGE22_PRIVATE_TRAFFIC = FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
