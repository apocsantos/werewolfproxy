#!/usr/bin/env python3
"""Disposable Stage18-to-Stage19 software-upgrade and downgrade exercise."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import os
import shutil
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
STAGE18_HARNESS = ROOT / "tests/stage18_cli/provisioning.py"


def load_stage18():
    spec = importlib.util.spec_from_file_location("stage18_provisioning", STAGE18_HARNESS)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load Stage18 provisioning harness")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def digest_tree(root: Path) -> dict[str, str]:
    result = {}
    for path in sorted(root.rglob("*")):
        if path.is_file() and path.name != ".den.lock":
            result[path.relative_to(root).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def atomic_replace(source: Path, destination: Path) -> None:
    staged = destination.with_name(destination.name + ".stage19-new")
    shutil.copyfile(source, staged)
    os.chmod(staged, 0o755)
    os.replace(staged, destination)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage18-daemon", type=Path, required=True)
    parser.add_argument("--stage18-control", type=Path, required=True)
    parser.add_argument("--stage19-daemon", type=Path, required=True)
    parser.add_argument("--stage19-control", type=Path, required=True)
    parser.add_argument("--tmp-parent", type=Path, required=True)
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    h = load_stage18()
    h.private(args.tmp_parent)
    root = Path(tempfile.mkdtemp(prefix="stage19-upgrade-", dir=args.tmp_parent))
    h.private(root)
    installed = root / "installed"; installed.mkdir(mode=0o755)
    daemon = installed / "werewolfd"; control = installed / "werewolfctl"
    atomic_replace(args.stage18_daemon.resolve(), daemon)
    atomic_replace(args.stage18_control.resolve(), control)
    echo = h.Echo(); echo.start()
    a = h.Node(root, "a", daemon, control)
    b = h.Node(root, "b", daemon, control)
    try:
        a.start(); b.start()
        a_public, b_public = h.exchange(a, b)
        a.run_ctl("target", "allow", b_public["fingerprint"], f"127.0.0.1:{echo.port}")
        fang_port = h.free_port(h.socket.SOCK_STREAM)
        b.run_ctl("fang", "create", "upgrade-route", "a", f"127.0.0.1:{fang_port}",
                  f"127.0.0.1:{echo.port}", "--transport", "tcp")
        b.run_ctl("fang", "activate", "upgrade-route")
        h.require(h.send_echo(fang_port, b"stage19-upgrade") == b"stage19-upgrade",
                  "Stage18 baseline forwarding failed")
        b.run_ctl("silver", "on")
        h.require(b.run_ctl("status")["silver"] == "active", "Silver did not persist as locked")
        a.stop(); b.stop()
        before = {"a": digest_tree(a.den), "b": digest_tree(b.den)}
        active_intent = (b.den / "active_fangs.json").read_bytes()
        pelt = (a.den / "pelt.json").read_bytes()
        pack = (a.den / "pack.json").read_bytes()
        targets = (a.den / "target_policy.json").read_bytes()
        print("PASS Stage18 baseline state and Silver lock recorded", flush=True)

        atomic_replace(args.stage19_daemon.resolve(), daemon)
        atomic_replace(args.stage19_control.resolve(), control)
        a.start(); b.start()
        h.require(b.run_ctl("status")["silver"] == "active", "upgrade unlocked Silver")
        h.require((b.den / "active_fangs.json").read_bytes() == active_intent,
                  "upgrade changed active Fang intent")
        a.stop(); b.stop()
        h.require({"a": digest_tree(a.den), "b": digest_tree(b.den)} == before,
                  "Stage19 software upgrade altered protected persistent state")
        print("PASS Stage19 upgrade preserves Pelt, Pack, targets, Silver, and Fang intent", flush=True)

        atomic_replace(args.stage18_daemon.resolve(), daemon)
        atomic_replace(args.stage18_control.resolve(), control)
        a.start(); b.start()
        h.require(b.run_ctl("status")["silver"] == "active", "Stage18 downgrade rejected or unlocked state")
        h.require((a.den / "pelt.json").read_bytes() == pelt, "downgrade changed Pelt")
        h.require((a.den / "pack.json").read_bytes() == pack, "downgrade changed Pack")
        h.require((a.den / "target_policy.json").read_bytes() == targets,
                  "downgrade changed target policy")
        b.run_ctl("silver", "off")
        b.run_ctl("fang", "activate", "upgrade-route")
        h.require(h.send_echo(fang_port, b"stage19-downgrade") == b"stage19-downgrade",
                  "Stage18 software downgrade forwarding failed")
        print("PASS software downgrade retains compatible state and forwarding", flush=True)
        print("STAGE19_UPGRADE_HARNESS = PASS")
        return 0
    finally:
        a.destroy(); b.destroy(); echo.close()
        if args.keep:
            print(f"kept {root}")
        else:
            shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"STAGE19_UPGRADE_HARNESS = FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
