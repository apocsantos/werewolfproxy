#!/usr/bin/env python3
"""Disposable Stage18 CLI-only provisioning and administration checks."""
from __future__ import annotations

import argparse
import json
import os
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

DEADLINE = 8.0


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


def wait_for(predicate, label: str, process: subprocess.Popen[bytes] | None = None) -> None:
    end = time.monotonic() + DEADLINE
    last: Exception | None = None
    while time.monotonic() < end:
        if process is not None and process.poll() is not None:
            raise RuntimeError(f"{label}: daemon exited {process.returncode}")
        try:
            if predicate():
                return
        except (OSError, subprocess.CalledProcessError, RuntimeError) as error:
            last = error
        time.sleep(0.03)
    raise RuntimeError(f"timed out waiting for {label}: {last}")


class Echo:
    def __init__(self) -> None:
        self.port = free_port(socket.SOCK_STREAM)
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)

    def start(self) -> None:
        self.thread.start()

    def run(self) -> None:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(("127.0.0.1", self.port))
            listener.listen()
            listener.settimeout(0.1)
            while not self.stop.is_set():
                try:
                    client, _ = listener.accept()
                except TimeoutError:
                    continue
                with client:
                    data = client.recv(4096)
                    if data:
                        client.sendall(data)

    def close(self) -> None:
        self.stop.set()
        self.thread.join(timeout=DEADLINE)


class Node:
    def __init__(self, root: Path, name: str, daemon: Path, ctl: Path) -> None:
        self.root, self.name, self.daemon, self.ctl = root, name, daemon, ctl
        self.den = root / f"{name}-den"
        self.runtime = root / f"{name}-runtime"
        private(self.runtime)
        self.control = self.runtime / "control.sock"
        self.tcp = free_port(socket.SOCK_STREAM)
        self.quic = free_port(socket.SOCK_DGRAM)
        self.process: subprocess.Popen[bytes] | None = None
        self.log = None
        self.run_ctl("--home", str(self.den), "den", "init")

    def run_ctl(self, *args: str, expect: int = 0) -> dict:
        output = subprocess.run(
            [str(self.ctl), "--socket", str(self.control), "--json", *args],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=DEADLINE,
        )
        require(output.returncode == expect, f"{self.name} ctl {' '.join(args)} exit {output.returncode}: {output.stderr.decode(errors='replace')}")
        if not output.stdout:
            return {}
        data = json.loads(output.stdout)
        if expect == 0:
            require(data.get("ok") is True, f"{self.name} ctl did not report success")
            return data["result"]
        return data

    def start(self) -> None:
        require(self.process is None or self.process.poll() is not None, "node already running")
        self.log = (self.root / f"{self.name}.log").open("ab")
        self.process = subprocess.Popen(
            [str(self.daemon), "--home", str(self.den), "--socket", str(self.control),
             "--listen", f"127.0.0.1:{self.tcp}", "--quic-listen", f"127.0.0.1:{self.quic}"],
            stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True,
        )
        wait_for(lambda: self.control.is_socket(), f"{self.name} control socket", self.process)
        wait_for(lambda: self.run_ctl("status"), f"{self.name} status", self.process)

    def stop(self) -> None:
        if self.process is not None and self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            require(self.process.wait(timeout=DEADLINE) == 0, f"{self.name} stop failed")
        if self.log is not None:
            self.log.close()
            self.log = None
        require(not self.control.exists(), f"{self.name} left control socket")

    def destroy(self) -> None:
        if self.process is not None and self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=DEADLINE)
        if self.log is not None:
            self.log.close()


def exchange(node_a: Node, node_b: Node) -> tuple[dict, dict]:
    node_a.run_ctl("pelt", "init")
    node_b.run_ctl("pelt", "init")
    a_public = node_a.run_ctl("pelt", "show")
    b_public = node_b.run_ctl("pelt", "show")
    require("secret" not in json.dumps(a_public).lower(), "Pelt show exposed secret field")
    node_a.run_ctl("state", "manifest-migrate")
    node_b.run_ctl("state", "manifest-migrate")
    node_a.run_ctl("silver", "off")
    node_b.run_ctl("silver", "off")
    node_a.run_ctl("pack", "add", "b", b_public["public_key_b64"], f"tcp://127.0.0.1:{node_b.tcp}")
    node_b.run_ctl("pack", "add", "a", a_public["public_key_b64"], f"tcp://127.0.0.1:{node_a.tcp}")
    return a_public, b_public


def send_echo(port: int, payload: bytes = b"stage18") -> bytes:
    with socket.create_connection(("127.0.0.1", port), timeout=DEADLINE) as client:
        client.sendall(payload)
        return client.recv(len(payload))


def denied_forward(port: int, payload: bytes) -> None:
    """A refused secure handshake may close cleanly or reset the local Fang."""
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=DEADLINE) as client:
            client.settimeout(DEADLINE)
            client.sendall(payload)
            require(client.recv(16) == b"", "denied request forwarded application data")
    except ConnectionResetError:
        pass


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--daemon", type=Path, required=True)
    parser.add_argument("--control", type=Path, required=True)
    parser.add_argument("--tmp-parent", type=Path, required=True)
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    private(args.tmp_parent)
    root = Path(tempfile.mkdtemp(prefix="stage18-", dir=args.tmp_parent))
    private(root)
    echo = Echo(); echo.start()
    a = Node(root, "a", args.daemon.resolve(), args.control.resolve())
    b = Node(root, "b", args.daemon.resolve(), args.control.resolve())
    try:
        a.start(); b.start()
        a_public, b_public = exchange(a, b)
        print("PASS  bootstrap, Pelt, Pack, Silver, and manifest migration", flush=True)

        a.run_ctl("target", "allow", b_public["fingerprint"], f"127.0.0.1:{echo.port}")
        grants = a.run_ctl("target", "list")
        require(grants and grants[0]["targets"] == [f"127.0.0.1:{echo.port}"], "target grant missing")
        fang_port = free_port(socket.SOCK_STREAM)
        b.run_ctl("fang", "create", "route", "a", f"127.0.0.1:{fang_port}", f"127.0.0.1:{echo.port}", "--transport", "tcp")
        require(len(b.run_ctl("fang", "list")) == 1, "Fang profile missing")
        b.run_ctl("fang", "activate", "route")
        require(send_echo(fang_port) == b"stage18", "CLI-provisioned secure forward failed")
        print("PASS  target and Fang CLI-only forwarding", flush=True)

        # Persisted activation intent must survive a clean daemon restart.
        b.stop(); b.start()
        wait_for(lambda: send_echo(fang_port) == b"stage18", "restored active Fang", b.process)
        b.run_ctl("fang", "deactivate", "route")
        b.stop(); b.start()
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
            probe.settimeout(0.2)
            require(probe.connect_ex(("127.0.0.1", fang_port)) != 0, "deactivated Fang restored")
        print("PASS  Fang activation/deactivation restart intent", flush=True)

        # Concurrent target mutations are serialized by the daemon coordinator.
        ports = [free_port(socket.SOCK_STREAM), free_port(socket.SOCK_STREAM)]
        workers = [subprocess.Popen([str(args.control), "--socket", str(a.control), "--json", "target", "allow", b_public["fingerprint"], f"127.0.0.1:{port}"], stdout=subprocess.PIPE, stderr=subprocess.PIPE) for port in ports]
        for worker in workers:
            stdout, stderr = worker.communicate(timeout=DEADLINE)
            require(worker.returncode == 0 and json.loads(stdout).get("ok"), f"concurrent target mutation failed: {stderr.decode()}")
        listed = a.run_ctl("target", "list")
        require(len(listed[0]["targets"]) == 3, "concurrent target mutation lost an update")
        print("PASS  concurrent protected-state mutation serialization", flush=True)

        # Silver fences an active forwarding listener; reopening remains explicit.
        b.run_ctl("fang", "activate", "route")
        b.run_ctl("silver", "on")
        require(b.run_ctl("status")["silver"] == "active", "Silver status did not lock")
        b.run_ctl("silver", "off")
        b.run_ctl("fang", "activate", "route")
        require(send_echo(fang_port) == b"stage18", "Silver unlock did not permit explicit activation")
        print("PASS  Silver CLI fence and recovery", flush=True)

        # Target revoke and Pack revoke make subsequent forwarding fail.
        b.run_ctl("fang", "deactivate", "route")
        a.run_ctl("target", "remove", b_public["fingerprint"], f"127.0.0.1:{echo.port}")
        b.run_ctl("fang", "activate", "route")
        denied_forward(fang_port, b"denied")
        b.run_ctl("fang", "deactivate", "route")
        a.run_ctl("target", "allow", b_public["fingerprint"], f"127.0.0.1:{echo.port}")
        a.run_ctl("pack", "revoke", "b")
        b.run_ctl("fang", "activate", "route")
        denied_forward(fang_port, b"revoked")
        print("PASS  target and Pack revocation through CLI", flush=True)

        a.stop(); b.stop()
        broken = root / "broken-den"; shutil.copytree(a.den, broken); os.chmod(broken, 0o700)
        (broken / "security_state_current.json").write_text("{", encoding="utf-8")
        os.chmod(broken / "security_state_current.json", 0o600)
        bad = subprocess.run([str(args.control), "--home", str(broken), "--json", "doctor"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=DEADLINE)
        require(bad.returncode != 0 and json.loads(bad.stdout).get("ok") is False, "doctor accepted corrupt selector")
        print("PASS  doctor reports stopped-Den manifest corruption", flush=True)
        print("STAGE18_CLI_PROVISIONING_HARNESS = PASS")
        return 0
    finally:
        a.destroy(); b.destroy(); echo.close()
        if args.keep:
            print(f"retained fixture: {root}")
        else:
            shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"STAGE18_CLI_PROVISIONING_HARNESS = FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
