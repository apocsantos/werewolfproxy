#!/usr/bin/env python3
"""Run the certified Stage21 archive through disposable local deployment tests.

Every daemon and Den is created in a private temporary directory. The script
verifies the pinned outer archive digest before extraction or installation.
It never builds from the checkout and never touches host system paths.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
EXPECTED_ARCHIVE_SHA256 = "e2d372739743b31359fc115d38f36c3db1c86ebec8afab026b244811c5cacea4"
EXPECTED_BINARY_SHA256 = {
    "werewolfd": "ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4",
    "werewolfctl": "c9c5e5cf17350b675927efd419e7bb970c3e136ff79977ce8101357e07f74818",
}
ROOT_NAME = "werewolfproxy-1.0.0-rc.1-linux-x86_64"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def run(argv: list[str], *, env: dict[str, str] | None = None,
        cwd: Path | None = None, timeout: int = 1800) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(argv, cwd=cwd, env=env, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, timeout=timeout, check=True)


def execute(name: str, argv: list[str], *, env: dict[str, str] | None = None,
            cwd: Path | None = None, timeout: int = 1800) -> bytes:
    print(f"RUN {name}", flush=True)
    try:
        result = run(argv, env=env, cwd=cwd, timeout=timeout)
    except subprocess.CalledProcessError as error:
        # Avoid emitting daemon logs or protocol details into reports. A bounded
        # tail is useful for diagnosis; the underlying harnesses redact Pelt.
        detail = (error.stdout + error.stderr)[-2000:].decode(errors="replace")
        raise RuntimeError(f"{name} failed (exit {error.returncode}): {detail}") from error
    print(f"PASS {name}", flush=True)
    return result.stdout


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--tmp-parent", type=Path, required=True,
                        help="private operator-owned directory, mode 0700")
    parser.add_argument("--report", type=Path,
                        help="optional sanitized JSON report; use an ignored/private path")
    parser.add_argument("--skip-stage17", action="store_true")
    parser.add_argument("--tcp-routes", type=int, default=10000)
    parser.add_argument("--quic-routes", type=int, default=10000)
    parser.add_argument("--transfer-mib", type=int, default=1024)
    parser.add_argument("--long-seconds", type=int, default=600)
    parser.add_argument("--idle-seconds", type=int, default=300)
    args = parser.parse_args()
    archive = args.archive.resolve(strict=True)
    tmp_parent = args.tmp_parent.resolve(strict=True)
    require(stat.S_IMODE(tmp_parent.stat().st_mode) == 0o700,
            "temporary parent must have mode 0700")

    actual_archive_hash = digest(archive)
    require(actual_archive_hash == EXPECTED_ARCHIVE_SHA256,
            f"certified archive digest mismatch: {actual_archive_hash}")
    print(f"PASS certified archive SHA-256 {actual_archive_hash}", flush=True)

    work = Path(tempfile.mkdtemp(prefix="stage22-local-", dir=tmp_parent))
    os.chmod(work, 0o700)
    report: dict[str, Any] = {
        "stage": "stage22-private-local",
        "result": "PASS",
        "archive_sha256": actual_archive_hash,
        "topology": "two isolated loopback Dens, independent generated identities",
        "environment": "single Linux x86_64 host; no external network, reboot, netem, or systemd mutation",
        "checks": [],
    }
    try:
        with tarfile.open(archive, "r:gz") as package:
            members = package.getmembers()
            require(all(member.name == ROOT_NAME or member.name.startswith(ROOT_NAME + "/")
                        for member in members), "archive root differs from certified candidate")
            require(all(not member.issym() and not member.islnk() for member in members),
                    "archive links are not accepted")
            package.extractall(work, filter="data")
        release = work / ROOT_NAME
        run(["sha256sum", "-c", "SHA256SUMS"], cwd=release)
        release_manifest = json.loads((release / "RELEASE-MANIFEST.json").read_text())
        require(release_manifest["version"] == "1.0.0-rc.1", "release version mismatch")
        require(release_manifest["artifact_sha256"]["bin/werewolfd"] == EXPECTED_BINARY_SHA256["werewolfd"]
                and release_manifest["artifact_sha256"]["bin/werewolfctl"] == EXPECTED_BINARY_SHA256["werewolfctl"],
                "embedded release manifest binary digests differ from certified binaries")
        require(release_manifest["cargo_lock_sha256"] ==
                "fbc16d90daaf58ba7f3f32f10615f8521103a5c019f98132ee37c36928458704",
                "embedded release manifest lock digest differs from Stage21")
        report["checks"].append("outer archive and inner SHA256SUMS verified")

        # Exercise the archive's own installer with a temporary DESTDIR.
        stage_root = work / "install-root"
        stage_root.mkdir(mode=0o700)
        install_env = dict(os.environ, DESTDIR=str(stage_root))
        execute("staged install from certified archive", [str(release / "install.sh")],
                cwd=release, env=install_env)
        daemon = stage_root / "usr/local/bin/werewolfd"
        control = stage_root / "usr/local/bin/werewolfctl"
        for label, binary in (("werewolfd", daemon), ("werewolfctl", control)):
            require(digest(binary) == EXPECTED_BINARY_SHA256[label],
                    f"installed {label} digest differs from certified binary")
            execute(f"installed {label} version outside checkout", [str(binary), "--version"], cwd=work)
        report["checks"].append("archive installer staged exact binaries; no host paths modified")

        # Stage19 validates inventory, checksums, failure-safe install,
        # uninstall preservation, and operation without a checkout.
        execute("Stage19 archive/install regression", [
            sys.executable, "-B", str(ROOT / "tests/stage19_release/release.py"),
            "--archive", str(archive),
        ], cwd=ROOT, timeout=1800)
        execute("Stage19 candidate reinstall and rollback characterization", [
            sys.executable, "-B", str(ROOT / "tests/stage19_release/upgrade.py"),
            "--stage18-daemon", str(daemon), "--stage18-control", str(control),
            "--stage19-daemon", str(daemon), "--stage19-control", str(control),
            "--tmp-parent", str(tmp_parent),
        ], cwd=ROOT, timeout=1800)
        report["checks"].append("Stage19 archive install/uninstall and same-version candidate reinstall/rollback state characterization")

        if not args.skip_stage17:
            execute("Stage17 lifecycle on extracted release binaries", [
                sys.executable, "-B", str(ROOT / "tests/stage17_lifecycle/lifecycle.py"),
                "--daemon", str(daemon), "--control", str(control), "--tmp-parent", str(tmp_parent),
            ], cwd=ROOT, timeout=1800)
            report["checks"].append("Stage17 process lifecycle harness")

        execute("Stage18 two-node provisioning on installed release binaries", [
            sys.executable, "-B", str(ROOT / "tests/stage18_cli/provisioning.py"),
            "--daemon", str(daemon), "--control", str(control), "--tmp-parent", str(tmp_parent),
        ], cwd=ROOT, timeout=1800)
        report["checks"].append("Stage18 identity, Pack, target, Fang, TCP, restart, Silver, revocation, doctor")

        chaos_output = execute("Stage20 CI-safe chaos on installed release binaries", [
            sys.executable, "-B", str(ROOT / "tests/stage20_chaos/soak.py"),
            "--profile", "ci", "--seed", "20260922", "--daemon", str(daemon),
            "--control", str(control), "--tmp-parent", str(tmp_parent),
        ], cwd=ROOT, timeout=2400)
        chaos = json.loads(chaos_output)
        require(chaos.get("result") == "PASS", "Stage20 CI-safe chaos result was not PASS")
        report["stage20_ci"] = chaos
        report["checks"].append("Stage20 CI-safe chaos including 50 MiB TCP/QUIC and resource quiescence")

        traffic_output = execute("Stage22 operational route churn, transfer, and session campaign", [
            sys.executable, "-B", str(ROOT / "tests/stage22_private/traffic.py"),
            "--daemon", str(daemon), "--control", str(control), "--tmp-parent", str(tmp_parent),
            "--tcp-routes", str(args.tcp_routes), "--quic-routes", str(args.quic_routes),
            "--transfer-mib", str(args.transfer_mib), "--long-seconds", str(args.long_seconds),
            "--idle-seconds", str(args.idle_seconds),
        ], cwd=ROOT, timeout=args.long_seconds * 2 + 7200)
        traffic_records = [line for line in traffic_output.decode().splitlines()
                           if line.startswith("{")]
        require(traffic_records, "Stage22 traffic harness emitted no JSON result")
        traffic = json.loads(traffic_records[-1])
        require(traffic.get("result") == "PASS", "Stage22 traffic result was not PASS")
        report["traffic"] = traffic
        report["checks"].append("Stage22 TCP/QUIC churn, integrity transfers, long-lived sessions, concurrent Fangs")
        report["installed_binary_sha256"] = EXPECTED_BINARY_SHA256

        if args.report:
            destination = args.report.resolve()
            destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            destination.write_text(json.dumps(report, sort_keys=True, indent=2) + "\n", encoding="utf-8")
            os.chmod(destination, 0o600)
        print(json.dumps(report, sort_keys=True), flush=True)
        print("STAGE22_PRIVATE_LOCAL_CAMPAIGN = PASS", flush=True)
        return 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError,
            tarfile.TarError, subprocess.CalledProcessError) as error:
        print(f"STAGE22_PRIVATE_LOCAL_CAMPAIGN = FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
