#!/usr/bin/env python3
"""Capture only production TCP forwarding-frame sizes for Stage13C modeling.

The daemon's existing local length-only diagnostics identify payload sizes.
The transparent relay supplies outer TLS record sizes. No payload, key,
decrypted application bytes, or raw wire capture is retained.
"""

import argparse
import collections
import csv
import json
import os
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests" / "stage13_traffic_morphology"))
import capture as base  # noqa: E402
import analyze as baseline_analyze  # noqa: E402

CLIENT_FRAME = re.compile(rb"TCPV2 client->server plaintext=(\d+) counter=(\d+)")
SERVER_FRAME = re.compile(rb"TCPV2 server->client plaintext=(\d+) counter=(\d+)")
FRAME_FIELDS = (
    "connection_id", "workload", "class", "direction", "payload_bytes",
    "padded_plaintext_bytes", "ciphertext_bytes", "inner_submitted_bytes",
    "outer_tls_record_bytes", "final_frame", "frame_count",
)
SESSION_FIELDS = (
    "id", "workload", "sample", "total_c2s", "total_s2c",
    "tls_record_count", "duration_ms", "time_to_first_response_ms",
    "frame_count_c2s", "frame_count_s2c", "frame_wire_c2s", "frame_wire_s2c",
    "nonforward_tls_c2s", "nonforward_tls_s2c",
    "tls_public_handshake_bytes", "tls_encrypted_handshake_bytes",
    "stage10_handshake_bytes", "close_record_bytes",
)


def log_path(lab, wolf):
    label = f"{wolf}-{lab.generation}"
    matches = [pathlib.Path(row["log"]) for row in lab.commands if row["label"] == label]
    base.require(len(matches) == 1, f"missing {label} daemon log")
    return matches[0]


def lengths_since(path, offset, pattern):
    data = path.read_bytes()[offset:]
    found = [(int(size), int(counter)) for size, counter in pattern.findall(data)]
    base.require([counter for _, counter in found] == list(range(len(found))),
                 "frame counter gap or mixed connection logs")
    return [size for size, _ in found]


def allowed_current_padded(payload):
    minimum = max(payload + 2, 768)
    base.require(payload <= 2046, "production payload exceeds frame limit")
    return range(minimum, 2049, 128)


def classify_nonforward(records):
    # The first type-23 record in each direction is the encrypted TLS
    # handshake. The next short records are Stage10 challenge/OPEN/ACK;
    # 24-byte records are close. This ordering is fixed by the actual
    # production call path and validated for every captured connection.
    totals = dict(tls_public=0, tls_encrypted=0, stage10=0, close=0)
    for direction in ("c2s", "s2c"):
        side = [(size, kind) for _, label, size, kind in records if label == direction]
        public = [size for size, kind in side if kind in (20, 22)]
        short_app = [size for size, kind in side if kind == 23 and size < 810]
        base.require(len(short_app) == (3 if direction == "c2s" else 4),
                     "unexpected number of encrypted handshake/Stage10/close records")
        base.require(short_app[-1] == 24, "missing final close record")
        base.require(short_app[0] < 810, "TLS encrypted handshake size")
        totals["tls_public"] += sum(public)
        totals["tls_encrypted"] += short_app[0]
        totals["stage10"] += sum(short_app[1:-1])
        totals["close"] += short_app[-1]
    return totals


def extract(session, client_lengths, server_lengths):
    payloads = {"c2s": client_lengths, "s2c": server_lengths}
    frames = []
    frame_wires = {}
    nonforward = {}
    for direction in ("c2s", "s2c"):
        side = [(size, kind) for _, label, size, kind in session.records
                if label == direction]
        application = [size for size, kind in side if kind == 23 and size >= 810]
        base.require(len(application) == len(payloads[direction]),
                     f"{direction} production payload/TLS record count mismatch")
        for index, (payload, outer) in enumerate(zip(payloads[direction], application)):
            padded = outer - 42  # 4 inner length +16 AEAD +22 TLS overhead
            base.require(padded in allowed_current_padded(payload),
                         f"{direction} TLS size cannot encode production padding")
            frames.append({
                "connection_id": session.id, "workload": session.workload,
                "class": baseline_analyze.WORKLOAD_CLASS[session.workload],
                "direction": direction, "payload_bytes": payload,
                "padded_plaintext_bytes": padded,
                "ciphertext_bytes": padded + 16,
                "inner_submitted_bytes": padded + 20,
                "outer_tls_record_bytes": outer,
                "final_frame": int(index == len(application) - 1),
            })
        frame_wires[direction] = sum(application)
        nonforward[direction] = sum(size for size, _ in side) - frame_wires[direction]
    base.require(sum(size for _, _, size, _ in session.records) ==
                 sum(size for _, _, size in session.units),
                 "TLS records do not account for complete relay byte stream")
    parts = classify_nonforward(session.records)
    base.require(sum(parts.values()) == sum(nonforward.values()),
                 "handshake/close classification misses nonforward records")
    return frames, frame_wires, nonforward, parts


def write_csv(path, fields, rows):
    pending = path.with_name(path.name + ".pending")
    with pending.open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    pending.replace(path)


def run(args):
    os.umask(0o077)
    output = pathlib.Path(args.output_dir)
    output.mkdir(parents=True, exist_ok=True)
    lab = base.Lab()
    print(f"Disposable lab: {lab.base}", flush=True)
    target = base.Target()
    relay = base.TcpRelay(("127.0.0.1", 1))
    udp = base.UdpRelay(relay.port, ("127.0.0.1", 1))
    sessions = []
    grouped = []
    try:
        base.setup(lab, target, relay, udp)
        relay.upstream = ("127.0.0.1", lab.ports["a_tcp"])
        udp.upstream = ("127.0.0.1", lab.ports["a_quic"])
        for workload in base.WORKLOADS:
            count = args.bulk_samples if workload == "bulk_50mib" else args.samples
            for sample in range(count):
                session = base.Session("tcp", workload, sample)
                task = base.TargetTask(workload)
                target.tasks.put(task)
                logs = {wolf: log_path(lab, wolf) for wolf in ("a", "b")}
                offsets = {wolf: path.stat().st_size for wolf, path in logs.items()}
                relay.active = session
                try:
                    base.application_workload(lab.ports["tcp_fang"], task, session)
                    base.require(task.accepted.wait(20), "target never accepted")
                    session.target_accept_ns = task.accept_ns
                    base.require(session.bridge_done.wait(20), "TCP relay did not close")
                    base.require(task.done.wait(20), "target task did not close")
                    base.require(task.error is None, f"target error: {task.error}")
                    client_lengths = lengths_since(logs["b"], offsets["b"], CLIENT_FRAME)
                    server_lengths = lengths_since(logs["a"], offsets["a"], SERVER_FRAME)
                    frames, frame_wires, nonforward, parts = extract(
                        session, client_lengths, server_lengths)
                    key_counts = collections.Counter(
                        (frame["connection_id"], frame["workload"], frame["class"],
                         frame["direction"], frame["payload_bytes"],
                         frame["padded_plaintext_bytes"], frame["ciphertext_bytes"],
                         frame["inner_submitted_bytes"], frame["outer_tls_record_bytes"],
                         frame["final_frame"])
                        for frame in frames)
                    grouped.extend(dict(zip(FRAME_FIELDS[:-1], key), frame_count=value)
                                   for key, value in sorted(key_counts.items()))
                    features = base.features(session)
                    sessions.append({
                        "id": session.id, "workload": workload, "sample": sample,
                        "total_c2s": features["total_c2s"],
                        "total_s2c": features["total_s2c"],
                        "tls_record_count": features["tls_record_count"],
                        "duration_ms": features["duration_ms"],
                        "time_to_first_response_ms": features[
                            "time_to_first_response_ms"],
                        "frame_count_c2s": len(client_lengths),
                        "frame_count_s2c": len(server_lengths),
                        "frame_wire_c2s": frame_wires["c2s"],
                        "frame_wire_s2c": frame_wires["s2c"],
                        "nonforward_tls_c2s": nonforward["c2s"],
                        "nonforward_tls_s2c": nonforward["s2c"],
                        "tls_public_handshake_bytes": parts["tls_public"],
                        "tls_encrypted_handshake_bytes": parts["tls_encrypted"],
                        "stage10_handshake_bytes": parts["stage10"],
                        "close_record_bytes": parts["close"],
                    })
                    write_csv(output / "baseline_frames.csv", FRAME_FIELDS, grouped)
                    write_csv(output / "baseline_sessions.csv", SESSION_FIELDS, sessions)
                finally:
                    relay.active = None
                print(f"PASS tcp {workload} {sample + 1}/{count}", flush=True)
        manifest = {
            "schema": 1,
            "source_head": base.subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            "lock_sha256": base.digest(ROOT / "Cargo.lock"),
            "connections": len(sessions), "frames": sum(row["frame_count"] for row in grouped),
            "grouped_rows": len(grouped),
            "workloads": {name: args.bulk_samples if name == "bulk_50mib" else args.samples
                          for name in base.WORKLOADS},
            "capture": "existing local payload-length diagnostics + passive TLS record lengths",
            "record_overhead": {"ciphertext": 16, "inner_length": 4, "outer_tls": 22},
        }
        base.write_json(output / "capture_manifest.json", manifest)
        print("COMPLETE", manifest["connections"], "connections,",
              manifest["frames"], "forwarding frames", flush=True)
    finally:
        relay.close()
        udp.close()
        target.close()
        lab.cleanup()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=30)
    parser.add_argument("--bulk-samples", type=int, default=5)
    parser.add_argument("--output-dir", default="tests/stage13_padding_study")
    run(parser.parse_args())
