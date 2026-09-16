#!/usr/bin/env python3
"""Disposable Linux process-lifecycle checks for werewolfd.

The harness intentionally uses a private parent beneath the invoking user's
home: Stage11B rejects a shared /tmp ancestry. It retains no fixture after a
successful run unless --keep is requested.
"""

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
import time
from pathlib import Path


DEADLINE = 8.0


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def private_dir(path: Path) -> None:
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(path, 0o700)
    mode = stat.S_IMODE(path.stat().st_mode)
    require(mode == 0o700, f"private mode required for {path}: {mode:o}")


def free_port(sock_type: int) -> int:
    with socket.socket(socket.AF_INET, sock_type) as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def wait_for(predicate, label: str, process: subprocess.Popen[bytes] | None = None) -> None:
    deadline = time.monotonic() + DEADLINE
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        if process is not None and process.poll() is not None:
            raise RuntimeError(f"{label}: daemon exited {process.returncode}")
        try:
            if predicate():
                return
        except (ConnectionError, OSError, subprocess.CalledProcessError) as error:
            last_error = error
        time.sleep(0.025)
    detail = f": {last_error}" if last_error is not None else ""
    raise RuntimeError(f"timed out waiting for {label}{detail}")


class Fixture:
    def __init__(self, root: Path, daemon: Path, control: Path) -> None:
        self.root = root
        self.daemon = daemon
        self.control = control
        self.den = root / "den"
        self.runtime = root / "runtime"
        private_dir(self.den)
        private_dir(self.runtime)
        (self.den / "silver.json").write_text('{"version":1,"mode":"open"}', encoding="utf-8")
        os.chmod(self.den / "silver.json", 0o600)
        self.socket = self.runtime / "control.sock"
        self.tcp_port = free_port(socket.SOCK_STREAM)
        self.quic_port = free_port(socket.SOCK_DGRAM)
        self.processes: list[subprocess.Popen[bytes]] = []

    def start(self, label: str, home: Path | None = None, socket_path: Path | None = None,
              tcp_port: int | None = None) -> subprocess.Popen[bytes]:
        home = home or self.den
        socket_path = socket_path or self.socket
        tcp_port = tcp_port or self.tcp_port
        log = self.root / f"{label}.log"
        handle = log.open("wb")
        process = subprocess.Popen(
            [str(self.daemon), "--home", str(home), "--socket", str(socket_path),
             "--listen", f"127.0.0.1:{tcp_port}", "--quic-listen", f"127.0.0.1:{self.quic_port}"],
            stdout=handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        process._stage17_log = handle  # type: ignore[attr-defined]
        self.processes.append(process)
        return process

    def finish(self, process: subprocess.Popen[bytes], expected: int) -> None:
        actual = process.wait(timeout=DEADLINE)
        process._stage17_log.close()  # type: ignore[attr-defined]
        require(actual == expected, f"daemon exit {actual}, expected {expected}")

    def terminate_leftovers(self) -> None:
        for process in self.processes:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=DEADLINE)
            process._stage17_log.close()  # type: ignore[attr-defined]

    def status(self) -> bool:
        subprocess.run(
            [str(self.control), "--socket", str(self.socket), "status"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=True,
            timeout=DEADLINE,
        )
        return True

    def ready(self, process: subprocess.Popen[bytes]) -> None:
        wait_for(lambda: self.socket.is_socket(), "control socket", process)
        wait_for(self.status, "local authenticated control", process)
        wait_for(
            lambda: socket.create_connection(("127.0.0.1", self.tcp_port), timeout=0.1).close() is None,
            "TCP listener",
            process,
        )

    def stop(self, process: subprocess.Popen[bytes], sig: signal.Signals) -> None:
        process.send_signal(sig)
        self.finish(process, 0)
        require(not self.socket.exists(), "normal shutdown left a control socket")


def cli(control: Path, socket_path: Path, *args: str) -> None:
    subprocess.run(
        [str(control), "--socket", str(socket_path), *args],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=True,
        timeout=DEADLINE,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--daemon", type=Path, required=True)
    parser.add_argument("--control", type=Path, required=True)
    parser.add_argument("--tmp-parent", type=Path, required=True)
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    require(args.daemon.is_file() and os.access(args.daemon, os.X_OK), "daemon is not executable")
    require(args.control.is_file() and os.access(args.control, os.X_OK), "control tool is not executable")
    private_dir(args.tmp_parent)
    root = Path(tempfile.mkdtemp(prefix="stage17-", dir=args.tmp_parent))
    private_dir(root)
    fixture = Fixture(root, args.daemon.resolve(), args.control.resolve())
    try:
        # First boot stays administratively available while Pelt is absent.
        first = fixture.start("first")
        wait_for(lambda: fixture.socket.is_socket(), "missing-Pelt control socket", first)
        cli(fixture.control, fixture.socket, "pelt", "init")
        wait_for(lambda: socket.create_connection(("127.0.0.1", fixture.tcp_port), timeout=0.1).close() is None,
                 "post-pelt TCP listener", first)
        cli(fixture.control, fixture.socket, "state", "manifest-migrate")
        fixture.stop(first, signal.SIGTERM)

        # SIGINT and SIGHUP retain the foreground process semantics.
        interrupt = fixture.start("sigint")
        fixture.ready(interrupt)
        fixture.stop(interrupt, signal.SIGINT)
        hup = fixture.start("sighup")
        fixture.ready(hup)
        hup.send_signal(signal.SIGHUP)
        time.sleep(0.1)
        require(hup.poll() is None, "SIGHUP terminated the daemon")
        fixture.status()
        fixture.stop(hup, signal.SIGTERM)

        # A live Den lock rejects a second daemon without disturbing the first.
        live = fixture.start("duplicate-live")
        fixture.ready(live)
        duplicate = fixture.start("duplicate")
        fixture.finish(duplicate, 66)
        fixture.status()
        fixture.stop(live, signal.SIGTERM)

        # A listener bind failure is a startup failure and cannot leave local
        # control active. The reserved socket is only a disposable test input.
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as occupied:
            occupied.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            occupied.bind(("127.0.0.1", 0))
            occupied.listen(1)
            failed_socket = fixture.runtime / "failed.sock"
            failed = fixture.start("bind-failure", socket_path=failed_socket,
                                   tcp_port=occupied.getsockname()[1])
            fixture.finish(failed, 69)
            require(not failed_socket.exists(), "bind failure left control socket active")

        # Repeated clean restarts retain a valid manifest generation and return
        # all runtime artifacts. Each status request uses kernel local credentials.
        for number in range(50):
            process = fixture.start(f"clean-{number}")
            fixture.ready(process)
            fixture.stop(process, signal.SIGTERM)

        # SIGKILL cannot clean up. The next process must reclaim only a refused
        # stale socket and reacquire the kernel-released flock, never a live one.
        for number in range(10):
            process = fixture.start(f"kill-{number}")
            fixture.ready(process)
            process.kill()
            fixture.finish(process, -signal.SIGKILL)
            require(fixture.socket.exists(), "SIGKILL unexpectedly removed socket")
        recovered = fixture.start("after-kill")
        fixture.ready(recovered)
        fixture.stop(recovered, signal.SIGTERM)

        # A malformed Stage15B selector remains fail closed before any control
        # socket or forwarding listener can become active.
        broken = root / "broken-den"
        shutil.copytree(fixture.den, broken)
        os.chmod(broken, 0o700)
        (broken / "security_state_current.json").write_text("{", encoding="utf-8")
        os.chmod(broken / "security_state_current.json", 0o600)
        broken_runtime = root / "broken-runtime"
        private_dir(broken_runtime)
        broken_socket = broken_runtime / "control.sock"
        failed = fixture.start("manifest-failure", home=broken, socket_path=broken_socket)
        fixture.finish(failed, 65)
        require(not broken_socket.exists(), "manifest failure activated control socket")

        print("STAGE17_LIFECYCLE_HARNESS = PASS")
        return 0
    finally:
        fixture.terminate_leftovers()
        if args.keep:
            print(f"retained fixture: {root}")
        else:
            shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"STAGE17_LIFECYCLE_HARNESS = FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
