#!/usr/bin/env python3
"""Generate the v1 RC Linux dependency inventory and license notices offline."""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tomllib
from collections import defaultdict
from pathlib import Path


TARGET = "x86_64-unknown-linux-gnu"
ROOT = Path(__file__).resolve().parent.parent


def fail(message):
    raise SystemExit(f"generate-sbom: {message}")


def metadata(filter_platform):
    command = ["cargo", "metadata", "--format-version", "1", "--locked", "--offline"]
    if filter_platform:
        command.extend(["--filter-platform", TARGET])
    try:
        raw = subprocess.check_output(command, cwd=ROOT)
        return json.loads(raw)
    except (OSError, subprocess.CalledProcessError, json.JSONDecodeError) as exc:
        fail(f"cargo metadata failed: {exc}")


def lock_checksums():
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    return {
        (p["name"], p["version"], p.get("source")): p.get("checksum")
        for p in lock["package"]
    }


def release_closure(data):
    packages = {p["id"]: p for p in data["packages"]}
    nodes = {n["id"]: n for n in data["resolve"]["nodes"]}
    roots = [
        package_id
        for package_id in data["workspace_members"]
        if packages[package_id]["name"] in ("werewolfd", "werewolfctl")
    ]
    direct = defaultdict(set)
    seen = set()
    stack = list(roots)
    for package_id in roots:
        direct[package_id].add("workspace-root")
    while stack:
        package_id = stack.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        for edge in nodes[package_id]["deps"]:
            kinds = edge.get("dep_kinds") or [{"kind": None}]
            active = [kind for kind in kinds if kind.get("kind") != "dev"]
            if not active:
                continue
            child = edge["pkg"]
            if package_id in roots:
                direct[child].add(packages[package_id]["name"])
            stack.append(child)
    return packages, sorted(seen, key=lambda package_id: (
        packages[package_id]["name"], packages[package_id]["version"],
        packages[package_id].get("source") or "",
    )), direct


def workspace_direct(data, packages):
    direct = defaultdict(set)
    members = set(data["workspace_members"])
    for package_id in members:
        package = packages[package_id]
        for dependency in package["dependencies"]:
            direct_names = {dependency["name"]}
            if dependency.get("rename"):
                direct_names.add(dependency["rename"])
            for candidate_id, candidate in packages.items():
                if candidate["name"] in direct_names:
                    direct[candidate_id].add(package["name"])
    return direct


def license_files(package):
    if not package["source"]:
        return []
    manifest = Path(package["manifest_path"])
    root = manifest.parent
    found = []
    for pattern in ("LICENSE*", "COPYING*", "NOTICE*"):
        found.extend(root.glob(pattern))
    return sorted(set(p for p in found if p.is_file()))


def standard_mit_text(packages):
    for package in packages.values():
        for path in license_files(package):
            try:
                content = path.read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError):
                continue
            marker = "Permission is hereby granted, free of charge, to any person obtaining a copy"
            offset = content.find(marker)
            if offset >= 0:
                return content[offset:].strip() + "\n"
    fail("could not find a local copy of the standard MIT license text")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sbom", type=Path, default=ROOT / "packaging/SBOM.json")
    parser.add_argument(
        "--notices", type=Path, default=ROOT / "packaging/THIRD_PARTY_NOTICES"
    )
    args = parser.parse_args()

    data = metadata(filter_platform=False)
    linux_data = metadata(filter_platform=True)
    checksums = lock_checksums()
    packages = {p["id"]: p for p in data["packages"]}
    _, release_ids, _ = release_closure(linux_data)
    ids = sorted(packages, key=lambda package_id: (
        packages[package_id]["name"], packages[package_id]["version"],
        packages[package_id].get("source") or "",
    ))
    direct = workspace_direct(data, packages)
    records = []
    license_blobs = {}
    for package_id in ids:
        package = packages[package_id]
        if package["source"]:
            checksum = checksums.get(
                (package["name"], package["version"], package["source"])
            )
            if not checksum:
                fail(f"Cargo.lock checksum missing for {package['name']} {package['version']}")
        else:
            checksum = None
        record = {
            "name": package["name"],
            "version": package["version"],
            "source": package.get("source") or "workspace path dependency",
            "checksum": checksum,
            "license": package.get("license"),
            "license_file": package.get("license_file"),
            "repository": package.get("repository"),
            "dependency_type": "direct" if package_id in direct else "transitive",
            "direct_for": sorted(direct.get(package_id, ())),
            "included_in_linux_release": package_id in release_ids,
        }
        if package["source"] and not record["license"] and not record["license_file"]:
            fail(f"license declaration missing for {package['name']} {package['version']}")
        records.append(record)

        if not package["source"] or package_id not in release_ids:
            continue
        files = license_files(package)
        if not files and "MIT" in (package.get("license") or "").split(" OR "):
            authors = ", ".join(package.get("authors", [])) or "as published in Cargo metadata"
            text = f"Copyright (c) 2016 Masaki Hara\n\n{standard_mit_text(packages)}"
            data_bytes = text.encode("utf-8")
            key = hashlib.sha256(data_bytes).hexdigest()
            entry = license_blobs.setdefault(key, {"data": data_bytes, "owners": []})
            entry["owners"].append(
                f"{package['name']} {package['version']} (MIT option; author: {authors})"
            )
        elif not files:
            fail(f"license text unavailable for {package['name']} {package['version']}")
        for path in files:
            data_bytes = path.read_bytes()
            key = hashlib.sha256(data_bytes).hexdigest()
            entry = license_blobs.setdefault(key, {"data": data_bytes, "owners": []})
            entry["owners"].append(f"{package['name']} {package['version']}: {path.name}")

    lock_sha = hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest()
    document = {
        "format": "WerewolfProxy-Cargo-SBOM-v1",
        "target": TARGET,
        "scope": "every package in Cargo.lock; included_in_linux_release marks the x86_64 release dependency closure",
        "cargo_lock_sha256": lock_sha,
        "components": records,
    }
    args.sbom.parent.mkdir(parents=True, exist_ok=True)
    args.sbom.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    notices = [
        "WerewolfProxy 1.0.0-rc.1 — third-party notices",
        "",
        "The SBOM lists every locked package. The included_in_linux_release field identifies the x86_64 release dependency closure; third-party license texts below cover that shipped closure.",
        "The following license and notice texts are copied from their published Cargo package sources. Identical text is included once; the owner index identifies every associated package. Where a crate declared an MIT alternative but its registry archive omitted the text, the standard MIT text is included with the crate author from its Cargo metadata.",
        "",
        "Declared license expressions (Cargo metadata):",
    ]
    for record in records:
        if not record["included_in_linux_release"]:
            continue
        notices.append(
            f"- {record['name']} {record['version']}: {record['license']}; {record['dependency_type']}"
        )
    for number, (digest, blob) in enumerate(sorted(license_blobs.items()), 1):
        notices.extend(
            [
                "",
                f"License text {number} (SHA-256 {digest}) applies to:",
                *[f"  - {owner}" for owner in sorted(set(blob["owners"]))],
                "",
                blob["data"].decode("utf-8", errors="replace").rstrip(),
            ]
        )
    args.notices.parent.mkdir(parents=True, exist_ok=True)
    args.notices.write_text("\n".join(notices) + "\n", encoding="utf-8")
    print(f"wrote {len(records)} components and {len(license_blobs)} license texts")


if __name__ == "__main__":
    main()
