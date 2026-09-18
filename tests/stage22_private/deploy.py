#!/usr/bin/env python3
"""Verify and stage the certified private artifact on configured Linux nodes.

Configuration is operator-owned and must be mode 0600. This tool never reads
private identities or credentials: SSH authentication comes from the caller's
existing SSH agent/configuration, and Pelt/Pack provisioning remains an
explicit operator action.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import stat
import subprocess
import sys
import tarfile
from pathlib import Path
from typing import Any

EXPECTED_ARCHIVE_SHA256 = "e2d372739743b31359fc115d38f36c3db1c86ebec8afab026b244811c5cacea4"
EXPECTED_DAEMON_SHA256 = "ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4"
EXPECTED_CONTROL_SHA256 = "c9c5e5cf17350b675927efd419e7bb970c3e136ff79977ce8101357e07f74818"
VERSION = "1.0.0-rc.1"
ROOT_NAME = f"werewolfproxy-{VERSION}-linux-x86_64"
FORBIDDEN_FRAGMENTS = (
    "password", "token", "private", "secret", "credential", "pelt", "identity",
    "sshkey", "keyfile", "keypath",
)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_no_secrets(value: Any, path: str = "config") -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            normalized = str(key).lower().replace("-", "").replace("_", "")
            require(not any(fragment in normalized for fragment in FORBIDDEN_FRAGMENTS),
                    f"forbidden credential/private-material field: {path}.{key}")
            check_no_secrets(item, f"{path}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            check_no_secrets(item, f"{path}[{index}]")
    elif isinstance(value, str):
        lowered = value.lower()
        has_pem_private_key = re.search(r"-----begin [a-z0-9 ]*private key-----", lowered) is not None
        require(not has_pem_private_key and "pelt.json" not in lowered,
                f"private key material or Pelt filename is forbidden in {path}")


def read_config(path: Path) -> dict[str, Any]:
    require(path.is_file(), f"configuration missing: {path}")
    require(stat.S_IMODE(path.stat().st_mode) == 0o600,
            f"configuration must have mode 0600: {path}")
    config = json.loads(path.read_text(encoding="utf-8"))
    check_no_secrets(config)
    require(config.get("artifact_sha256") == EXPECTED_ARCHIVE_SHA256,
            "configuration artifact digest does not match the certified Stage21 archive")
    nodes = config.get("nodes")
    require(isinstance(nodes, list) and len(nodes) >= 2,
            "configuration must define at least two nodes")
    names: set[str] = set()
    for node in nodes:
        require(isinstance(node, dict), "each node must be an object")
        required = ("name", "host", "ssh_destination", "den_path", "runtime_path",
                    "tcp_listen", "quic_listen", "test_target", "role")
        require(all(isinstance(node.get(key), str) and node[key] for key in required),
                "each node needs name, host, ssh_destination, Den/runtime paths, listeners, target, and role")
        require(node["name"] not in names, f"duplicate node name: {node['name']}")
        names.add(node["name"])
        for key in ("den_path", "runtime_path"):
            require(node[key].startswith("/") and node[key] != "/" and "\n" not in node[key],
                    f"{node['name']} {key} must be an absolute path")
        require(node["den_path"] != node["runtime_path"],
                f"{node['name']} Den and runtime paths must be independent")
        require(not node["ssh_destination"].startswith("-") and
                not any(char.isspace() for char in node["ssh_destination"]),
                f"{node['name']} ssh_destination must be a host alias or user@host")
    links = config.get("links", [])
    require(isinstance(links, list) and links, "links must define at least one two-node connection")
    adjacency = {name: set() for name in names}
    for link in links:
        require(isinstance(link, list) and len(link) == 2 and
                all(isinstance(name, str) and name in names for name in link) and link[0] != link[1],
                f"invalid topology link: {link!r}")
        adjacency[link[0]].add(link[1])
        adjacency[link[1]].add(link[0])
    reached: set[str] = set()
    pending = [next(iter(names))]
    while pending:
        current = pending.pop()
        if current not in reached:
            reached.add(current)
            pending.extend(adjacency[current] - reached)
    require(reached == names, "topology links must connect every configured node")
    return config


def verify_archive(path: Path) -> None:
    require(path.is_file(), f"archive missing: {path}")
    actual = sha256(path)
    require(actual == EXPECTED_ARCHIVE_SHA256,
            f"certified archive SHA-256 mismatch: {actual}")
    with tarfile.open(path, "r:gz") as archive:
        members = archive.getmembers()
        require(all(member.name == ROOT_NAME or member.name.startswith(ROOT_NAME + "/")
                    for member in members), "unexpected archive root")
        require(all(not member.issym() and not member.islnk() for member in members),
                "archive links are not accepted by the Stage22 staging harness")


def run(argv: list[str], *, input_data: bytes | None = None) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(argv, input=input_data, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, check=True)


def stage_node(node: dict[str, str], archive: Path, digest: str) -> None:
    remote = node["ssh_destination"]
    runtime = node["runtime_path"]
    remote_archive = f"{runtime}/stage22-{VERSION}.tar.gz"
    remote_release = f"{runtime}/releases/{ROOT_NAME}"
    # Paths and arguments are quoted independently. SSH destination is passed
    # as one argv item so shell metacharacters are not interpreted locally.
    run(["ssh", remote, "sh", "-s"], input_data=(
        f"set -eu\numask 077\nmkdir -p {shlex.quote(runtime)} "
        f"{shlex.quote(runtime + '/releases')}\nchmod 700 {shlex.quote(runtime)} "
        f"{shlex.quote(runtime + '/releases')}\n"
    ).encode())
    # Stream the small release archive over SSH; this avoids SCP's remote-path
    # parsing rules and keeps all private destination data in argv/config only.
    run(["ssh", remote, f"umask 077; cat > {shlex.quote(remote_archive)}"],
        input_data=archive.read_bytes())
    script = "\n".join((
        "set -eu",
        f"test \"$(sha256sum {shlex.quote(remote_archive)} | cut -d ' ' -f 1)\" = {shlex.quote(digest)}",
        f"mkdir -m 700 -p {shlex.quote(remote_release)}",
        f"tar -xzf {shlex.quote(remote_archive)} --strip-components=1 -C {shlex.quote(remote_release)}",
        f"cd {shlex.quote(remote_release)}",
        "sha256sum -c SHA256SUMS",
        f"mkdir -m 700 -p {shlex.quote(runtime + '/bin')}",
        f"test ! -L {shlex.quote(runtime + '/bin/werewolfd')}",
        f"test ! -L {shlex.quote(runtime + '/bin/werewolfctl')}",
        f"install -m 755 bin/werewolfd {shlex.quote(runtime + '/bin/werewolfd')}",
        f"install -m 755 bin/werewolfctl {shlex.quote(runtime + '/bin/werewolfctl')}",
        f"test \"$(sha256sum {shlex.quote(runtime + '/bin/werewolfd')} | cut -d ' ' -f 1)\" = {EXPECTED_DAEMON_SHA256}",
        f"test \"$(sha256sum {shlex.quote(runtime + '/bin/werewolfctl')} | cut -d ' ' -f 1)\" = {EXPECTED_CONTROL_SHA256}",
        f"test \"$({shlex.quote(runtime + '/bin/werewolfd')} --version)\"",
        f"test \"$({shlex.quote(runtime + '/bin/werewolfctl')} --version)\"",
        f"rm -f -- {shlex.quote(remote_archive)}",
    ))
    run(["ssh", remote, "sh", "-s"], input_data=script.encode())
    print(f"PASS staged verified artifact on {node['name']}", flush=True)


def check_node(node: dict[str, str]) -> None:
    remote = node["ssh_destination"]
    runtime = node["runtime_path"]
    ctl = runtime + "/bin/werewolfctl"
    socket_path = runtime + "/control.sock"
    output: list[dict[str, Any]] = []
    for action in ("status", "doctor"):
        remote_argv = [ctl, "--home", node["den_path"], "--socket", socket_path,
                       "--json", action]
        completed = run(["ssh", remote, shlex.join(remote_argv)])
        result = json.loads(completed.stdout)
        require(result.get("ok") is True, f"{node['name']} {action} reported failure")
        output.append(result)
    print(f"PASS {node['name']} status and doctor", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True,
                        help="operator-owned JSON file (mode 0600; keep outside Git)")
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--stage", action="store_true",
                        help="copy, verify, and stage the exact binaries using configured SSH destinations")
    parser.add_argument("--check", action="store_true",
                        help="run status and doctor on each node's configured Den/runtime")
    parser.add_argument("--plan", action="store_true",
                        help="validate inputs and print only node names and roles")
    args = parser.parse_args()
    config = read_config(args.config)
    verify_archive(args.archive)
    digest = sha256(args.archive)
    if args.plan:
        for node in config["nodes"]:
            print(f"VALID {node['name']} role={node['role']}")
        for left, right in config.get("links", []):
            print(f"LINK {left} <-> {right}")
    if args.stage:
        for node in config["nodes"]:
            stage_node(node, args.archive.resolve(), digest)
    if args.check:
        for node in config["nodes"]:
            check_node(node)
    if not args.plan and not args.stage and not args.check:
        parser.error("select --plan, --stage, or --check")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError,
            tarfile.TarError, subprocess.CalledProcessError) as error:
        print(f"STAGE22_DEPLOY = FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
