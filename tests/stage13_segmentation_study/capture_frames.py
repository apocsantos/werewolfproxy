#!/usr/bin/env python3
"""Capture ordered, metadata-only production TCP forwarding frames.

The source is the unchanged Stage13C production relay and daemon diagnostics.
Only payload lengths, measured padded lengths and sequence numbers are kept.
"""

import argparse
import csv
import gzip
import json
import os
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests" / "stage13_padding_study"))
import capture_frames as prior  # noqa: E402

base = prior.base
FIELDS = (
    "connection_id", "workload", "direction", "frame_seq", "payload_bytes",
    "padded_plaintext_bytes", "outer_tls_record_bytes", "final_frame",
)


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
    frame_total = 0
    try:
        base.setup(lab, target, relay, udp)
        relay.upstream = ("127.0.0.1", lab.ports["a_tcp"])
        udp.upstream = ("127.0.0.1", lab.ports["a_quic"])
        with gzip.open(output / "frame_dataset.csv.gz", "wt", newline="", compresslevel=9) as stream:
            writer = csv.DictWriter(stream, fieldnames=FIELDS, lineterminator="\n")
            writer.writeheader()
            for workload in base.WORKLOADS:
                count = args.bulk_samples if workload == "bulk_50mib" else args.samples
                for sample in range(count):
                    session = base.Session("tcp", workload, sample)
                    task = base.TargetTask(workload)
                    target.tasks.put(task)
                    logs = {wolf: prior.log_path(lab, wolf) for wolf in ("a", "b")}
                    offsets = {wolf: path.stat().st_size for wolf, path in logs.items()}
                    relay.active = session
                    try:
                        base.application_workload(lab.ports["tcp_fang"], task, session)
                        base.require(task.accepted.wait(20), "target never accepted")
                        session.target_accept_ns = task.accept_ns
                        base.require(session.bridge_done.wait(20), "TCP relay did not close")
                        base.require(task.done.wait(20), "target task did not close")
                        base.require(task.error is None, f"target error: {task.error}")
                        lengths = {
                            "c2s": prior.lengths_since(logs["b"], offsets["b"], prior.CLIENT_FRAME),
                            "s2c": prior.lengths_since(logs["a"], offsets["a"], prior.SERVER_FRAME),
                        }
                        frames, wire, nonforward, parts = prior.extract(
                            session, lengths["c2s"], lengths["s2c"])
                        sequence = {"c2s": 0, "s2c": 0}
                        for frame in frames:
                            direction = frame["direction"]
                            writer.writerow({
                                "connection_id": session.id, "workload": workload,
                                "direction": direction, "frame_seq": sequence[direction],
                                "payload_bytes": frame["payload_bytes"],
                                "padded_plaintext_bytes": frame["padded_plaintext_bytes"],
                                "outer_tls_record_bytes": frame["outer_tls_record_bytes"],
                                "final_frame": frame["final_frame"],
                            })
                            sequence[direction] += 1
                        features = base.features(session)
                        sessions.append({
                            "id": session.id, "workload": workload, "sample": sample,
                            "total_c2s": features["total_c2s"],
                            "total_s2c": features["total_s2c"],
                            "tls_record_count": features["tls_record_count"],
                            "duration_ms": features["duration_ms"],
                            "frame_count_c2s": len(lengths["c2s"]),
                            "frame_count_s2c": len(lengths["s2c"]),
                            "frame_wire_c2s": wire["c2s"],
                            "frame_wire_s2c": wire["s2c"],
                            "nonforward_tls_c2s": nonforward["c2s"],
                            "nonforward_tls_s2c": nonforward["s2c"],
                            "tls_public_handshake_bytes": parts["tls_public"],
                            "tls_encrypted_handshake_bytes": parts["tls_encrypted"],
                            "stage10_handshake_bytes": parts["stage10"],
                            "close_record_bytes": parts["close"],
                        })
                        frame_total += len(frames)
                    finally:
                        relay.active = None
                    print(f"PASS tcp {workload} {sample + 1}/{count}", flush=True)
        prior.write_csv(output / "sessions.csv", prior.SESSION_FIELDS, sessions)
        base.write_json(output / "capture_manifest.json", {
            "schema": 1,
            "source_head": base.subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            "lock_sha256": base.digest(ROOT / "Cargo.lock"),
            "connections": len(sessions), "frames": frame_total,
            "workloads": {name: args.bulk_samples if name == "bulk_50mib" else args.samples
                          for name in base.WORKLOADS},
            "retained": "ordered frame lengths and public wire metadata only",
        })
        print("COMPLETE", len(sessions), "connections,", frame_total, "frames", flush=True)
    finally:
        relay.close()
        udp.close()
        target.close()
        lab.cleanup()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=30)
    parser.add_argument("--bulk-samples", type=int, default=5)
    parser.add_argument("--output-dir", default="tests/stage13_segmentation_study")
    run(parser.parse_args())
