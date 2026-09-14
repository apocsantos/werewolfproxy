#!/usr/bin/env python3
"""Rechunk measured production TCP reads without changing Stage10 padding.

Online candidates split only bytes already returned by each observed read.
OFFLINE_IDEAL_MAX has complete future-stream knowledge and is only a bound.
"""

import collections
import csv
import gzip
import hashlib
import json
import pathlib
import random
import statistics
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / "tests" / "stage13_traffic_morphology"))
import analyze as baseline  # noqa: E402

NAMES = ("c2s", "s2c")


def read_data():
    with (HERE / "sessions.csv").open(newline="") as source:
        sessions = list(csv.DictReader(source))
    int_fields = (
        "sample", "total_c2s", "total_s2c", "tls_record_count",
        "frame_count_c2s", "frame_count_s2c", "frame_wire_c2s", "frame_wire_s2c",
        "nonforward_tls_c2s", "nonforward_tls_s2c",
    )
    for row in sessions:
        for key in int_fields:
            row[key] = int(row[key])
        row["duration_ms"] = float(row["duration_ms"])
    ordered = {row["id"]: {name: [] for name in NAMES} for row in sessions}
    final_indices = {row["id"]: {name: [] for name in NAMES} for row in sessions}
    with gzip.open(HERE / "frame_dataset.csv.gz", "rt", newline="") as source:
        for row in csv.DictReader(source):
            direction = row["direction"]
            assert direction in NAMES
            frames = ordered[row["connection_id"]][direction]
            assert int(row["frame_seq"]) == len(frames)
            n, padded, wire = (int(row[key]) for key in (
                "payload_bytes", "padded_plaintext_bytes", "outer_tls_record_bytes"))
            assert 0 < n <= 1400 and valid_padding(n, padded)
            assert wire == padded + 42
            frames.append((n, padded))
            if int(row["final_frame"]):
                final_indices[row["connection_id"]][direction].append(len(frames) - 1)
    for connection_id, directions in ordered.items():
        for direction in NAMES:
            expected = [len(directions[direction]) - 1] if directions[direction] else []
            assert final_indices[connection_id][direction] == expected
    return sessions, ordered


def valid_padding(payload, padded):
    minimum = max(payload + 2, 768)
    return minimum <= padded <= 2048 and (padded - minimum) % 128 == 0


def validate_control(sessions, ordered):
    frame_total = 0
    by_payload = collections.Counter()
    for row in sessions:
        app = baseline.APP_BYTES[row["workload"]]
        for direction, expected in zip(NAMES, app):
            frames = ordered[row["id"]][direction]
            assert sum(n for n, _ in frames) == expected, (row["id"], direction)
            assert len(frames) == row[f"frame_count_{direction}"]
            assert sum(p + 42 for _, p in frames) == row[f"frame_wire_{direction}"]
            assert row[f"nonforward_tls_{direction}"] + row[f"frame_wire_{direction}"] == row[f"total_{direction}"]
            frame_total += len(frames)
            by_payload.update(n for n, _ in frames)
        assert row["tls_record_count"] >= row["frame_count_c2s"] + row["frame_count_s2c"]
    manifest = json.loads((HERE / "capture_manifest.json").read_text())
    assert len(sessions) == manifest["connections"] and frame_total == manifest["frames"]
    assert manifest["source_head"] == "bc2d3d6955fa5125b11802252c7bf90c44c1c76b"
    assert manifest["lock_sha256"] == "c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17"
    return {"connections": len(sessions), "frames": frame_total,
            "exact_application_bytes_counts_and_wire_totals": True,
            "payload_size_histogram_top_ten": by_payload.most_common(10),
            "unique_payload_sizes": len(by_payload)}


def choose_cap(policy, frame_index, rng):
    kind = policy["kind"]
    if kind == "split_each_read":
        return policy["maximum_payload"]
    if kind == "alternate_each_read":
        return policy["capacities"][frame_index % len(policy["capacities"])]
    if kind == "random_each_read":
        return rng.choices(policy["capacities"], policy["probabilities"], k=1)[0]
    raise AssertionError(kind)


def split_stream(total, cap):
    result = [cap] * (total // cap)
    if total % cap:
        result.append(total % cap)
    return result


def segment(source, policy, rng):
    kind = policy["kind"]
    if kind == "observed":
        return list(source)
    if kind == "offline_direction":
        return [(n, None) for n in split_stream(
            sum(n for n, _ in source), policy["maximum_payload"])]
    result = []
    for read_size, observed_padding in source:
        remaining = read_size
        chunks = []
        while remaining:
            cap = choose_cap(policy, len(result), rng)
            amount = min(cap, remaining)
            chunks.append(amount)
            result.append((amount, None))
            remaining -= amount
        if len(chunks) == 1:
            # Common random numbers: an unchanged production read keeps its
            # measured padding draw, avoiding irrelevant Monte Carlo noise.
            result[-1] = (read_size, observed_padding)
    return result


def current_padding(payload, rng):
    minimum = max(payload + 2, 768)
    count = (2048 - minimum) // 128 + 1
    padded = minimum + 128 * rng.randrange(count)
    assert valid_padding(payload, padded)
    return padded


def classify(rows, names):
    previous = baseline.FEATURES["tcp"]
    try:
        baseline.FEATURES["tcp"] = tuple(names)
        return baseline.classify(rows, "tcp")
    finally:
        baseline.FEATURES["tcp"] = previous


def percentile(values, q):
    return baseline.percentile(values, q)


def median(values):
    return baseline.median(values)


def simulate(sessions, ordered, policy, seed):
    rng = random.Random(seed)
    rows = []
    payload_hist = collections.Counter()
    per_workload = collections.defaultdict(list)
    for original in sessions:
        item = {"id": original["id"], "workload": original["workload"],
                "sample": original["sample"], "duration_ms": original["duration_ms"]}
        original_frames = original["frame_count_c2s"] + original["frame_count_s2c"]
        for direction in NAMES:
            observed = ordered[original["id"]][direction]
            segmented = segment(observed, policy, rng)
            payloads = [n for n, _ in segmented]
            assert sum(payloads) == sum(n for n, _ in observed)
            assert all(0 < n <= 2046 for n in payloads)
            if payloads == [n for n, _ in observed]:
                paddings = [p for _, p in observed]
            else:
                paddings = [p if p is not None else current_padding(n, rng)
                            for n, p in segmented]
            frame_wire = sum(p + 42 for p in paddings)
            item[f"frame_count_{direction}"] = len(payloads)
            item[f"frame_wire_{direction}"] = frame_wire
            item[f"total_{direction}"] = original[f"nonforward_tls_{direction}"] + frame_wire
            item[f"app_{direction}"] = sum(payloads)
            item[f"payload_median_{direction}"] = median(payloads) or 0
            item[f"payload_p90_{direction}"] = percentile(payloads, .9) or 0
            item[f"last_payload_{direction}"] = payloads[-1] if payloads else 0
            payload_hist.update(payloads)
        item["frame_count_total"] = item["frame_count_c2s"] + item["frame_count_s2c"]
        item["tls_record_count"] = (original["tls_record_count"] - original_frames +
                                     item["frame_count_total"])
        item["app_total"] = item["app_c2s"] + item["app_s2c"]
        item["app_up_fraction"] = item["app_c2s"] / item["app_total"] if item["app_total"] else 0
        item["total_wire"] = item["total_c2s"] + item["total_s2c"]
        item["wire_up_fraction"] = item["total_c2s"] / item["total_wire"]
        if policy["kind"] == "observed":
            for name in NAMES:
                assert item[f"total_{name}"] == original[f"total_{name}"]
                assert item[f"frame_count_{name}"] == original[f"frame_count_{name}"]
        rows.append(item)
        per_workload[item["workload"]].append(item)
    return rows, payload_hist, per_workload


def policy_metrics(rows, original_by_id, hist, groups):
    sized = [r for r in rows if r["app_total"]]
    by_workload = {}
    for workload, members in groups.items():
        changes = []
        for row in members:
            before = original_by_id[row["id"]]
            changes.append({
                "frames": row["frame_count_total"] - before["frame_count_total"],
                "wire": row["total_wire"] - before["total_wire"],
                "frame_count": row["frame_count_total"],
                "c2s": row["frame_count_c2s"], "s2c": row["frame_count_s2c"],
                "wire_ratio": row["total_wire"] / row["app_total"] if row["app_total"] else None,
            })
        by_workload[workload] = {
            "connections": len(members),
            "median_frame_count": median([x["frame_count"] for x in changes]),
            "frame_count_variance": round(statistics.pvariance(x["frame_count"] for x in changes), 3),
            "median_c2s_frames": median([x["c2s"] for x in changes]),
            "median_s2c_frames": median([x["s2c"] for x in changes]),
            "median_added_frames": median([x["frames"] for x in changes]),
            "median_added_wire_bytes": median([x["wire"] for x in changes]),
            "median_wire_application_ratio": median([x["wire_ratio"] for x in changes
                                                        if x["wire_ratio"] is not None]),
        }
    total_frames = sum(hist.values())
    terminal_payloads = collections.Counter(
        r[f"last_payload_{name}"] for r in rows for name in NAMES
        if r[f"last_payload_{name}"])
    return {
        "total_frames": total_frames,
        "frame_count_by_direction": {name: sum(r[f"frame_count_{name}"] for r in rows)
                                     for name in NAMES},
        "total_estimated_wire_bytes": sum(r["total_wire"] for r in rows),
        "additional_estimated_wire_bytes": sum(
            r["total_wire"] - original_by_id[r["id"]]["total_wire"] for r in rows),
        "additional_frames": sum(r["frame_count_total"] -
                                 original_by_id[r["id"]]["frame_count_total"] for r in rows),
        "additional_aead_tag_bytes": 16 * sum(r["frame_count_total"] -
                                               original_by_id[r["id"]]["frame_count_total"] for r in rows),
        "additional_inner_prefix_bytes": 4 * sum(r["frame_count_total"] -
                                                  original_by_id[r["id"]]["frame_count_total"] for r in rows),
        "additional_estimated_tls_overhead_bytes": 22 * sum(r["frame_count_total"] -
                                                             original_by_id[r["id"]]["frame_count_total"] for r in rows),
        "unique_frame_payload_sizes": len(hist),
        "payload_size_top_ten": hist.most_common(10),
        "median_frame_payload": baseline.histogram_median(hist),
        "p90_frame_payload": weighted_percentile(hist, .9),
        "p95_frame_payload": weighted_percentile(hist, .95),
        "p99_frame_payload": weighted_percentile(hist, .99),
        "terminal_frame_fraction": round(sum(terminal_payloads.values()) / total_frames, 5),
        "terminal_payload_top_ten": terminal_payloads.most_common(10),
        "terminal_payload_below_current_1400_read_cap": sum(
            count for size, count in terminal_payloads.items() if size < 1400),
        "spearman_application_size_frame_count": baseline.spearman(
            [r["app_total"] for r in sized], [r["frame_count_total"] for r in sized]),
        "spearman_c2s_application_c2s_count": baseline.spearman(
            [r["app_c2s"] for r in sized], [r["frame_count_c2s"] for r in sized]),
        "spearman_s2c_application_s2c_count": baseline.spearman(
            [r["app_s2c"] for r in sized], [r["frame_count_s2c"] for r in sized]),
        "by_workload": by_workload,
    }


def weighted_percentile(counter, fraction):
    position = round((sum(counter.values()) - 1) * fraction)
    seen = 0
    for value, count in sorted(counter.items()):
        seen += count
        if seen > position:
            return value
    return None


def main():
    config = json.loads((HERE / "policies.json").read_text())
    sessions, ordered = read_data()
    validation = validate_control(sessions, ordered)
    original, _, _ = simulate(sessions, ordered, config["policies"][0], 13)
    original_by_id = {row["id"]: row for row in original}
    results = {}
    classifiers = {}
    for policy in config["policies"]:
        rows, hist, groups = simulate(sessions, ordered, policy, config["simulation_seed"])
        name = policy["name"]
        results[name] = policy_metrics(rows, original_by_id, hist, groups)
        classifiers[name] = {
            "full": baseline.classify(rows, "tcp"),
            "size_only": baseline.classify(rows, "tcp", size_only=True),
            **{key: classify(rows, fields)
               for key, fields in config["classifier_ablations"].items()},
        }
        print(name, results[name]["total_frames"],
              classifiers[name]["frame_count_only"]["accuracy"], flush=True)
    ideal = classifiers["OFFLINE_IDEAL_MAX"]
    majority = classifiers["A_current"]["full"]["majority_test_baseline"]
    floor = ideal["observed_wire_total_only"]["accuracy"]
    count_reference = classifiers["A_current"]["frame_count_only"]["accuracy"]
    count_gate = config["decision_rule"]["minimum_frame_count_accuracy_reduction_points"] / 100
    online_passing_first_gate = [
        policy["name"] for policy in config["policies"]
        if policy["online"] and policy["name"] != "A_current" and
        count_reference - classifiers[policy["name"]]["frame_count_only"]["accuracy"] >= count_gate
    ]
    if online_passing_first_gate:
        verdict = "MODEL_INCONCLUSIVE"  # Remaining cost/fingerprint gates need engineering review.
    elif floor - majority >= config["decision_rule"]["majority_floor_margin_points"] / 100:
        verdict = "SEGMENTATION_ONLY_INSUFFICIENT"
    else:
        verdict = "NO_ACCEPTABLE_SEGMENTATION_POLICY"
    summary = {
        "schema": 1,
        "source_head": "bc2d3d6955fa5125b11802252c7bf90c44c1c76b",
        "policy_definition_sha256": hashlib.sha256((HERE / "policies.json").read_bytes()).hexdigest(),
        "control_validation": validation,
        "majority_baseline": majority,
        "online_policies_passing_required_frame_count_classifier_gate": online_passing_first_gate,
        "policies": {p["name"]: p for p in config["policies"]},
        "candidate_metrics": results,
        "privacy_floor": {
            "offline_ideal_frame_count_accuracy": ideal["frame_count_only"]["accuracy"],
            "application_total_only_accuracy": ideal["application_byte_total_only"]["accuracy"],
            "directional_application_bytes_accuracy": ideal["directional_application_bytes_only"]["accuracy"],
            "ratio_only_accuracy": ideal["request_response_ratio_only"]["accuracy"],
            "observable_wire_total_only_accuracy": ideal["observed_wire_total_only"]["accuracy"],
            "observable_directional_wire_bytes_accuracy": ideal["directional_observed_wire_bytes_only"]["accuracy"],
            "offline_ideal_not_online_implementable": True,
        },
        "verdict": verdict,
        "recommended_next_stage": ("traffic-volume leakage and bounded shaping study" if
                                   verdict == "SEGMENTATION_ONLY_INSUFFICIENT" else
                                   "stop segmentation work"),
    }
    (HERE / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (HERE / "classifier_results.json").write_text(json.dumps(classifiers, indent=2) + "\n")
    (HERE / "privacy_floor.json").write_text(json.dumps(summary["privacy_floor"], indent=2) + "\n")
    with (HERE / "candidate_metrics.csv").open("w", newline="") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(("policy", "online", "frames", "added_frames", "wire_bytes",
                         "added_wire_bytes", "unique_payload_sizes", "app_frame_spearman",
                         "full_accuracy", "size_accuracy", "count_accuracy", "direction_count_accuracy"))
        for policy in config["policies"]:
            name = policy["name"]
            item = results[name]
            scores = classifiers[name]
            writer.writerow((name, int(policy["online"]), item["total_frames"],
                             item["additional_frames"], item["total_estimated_wire_bytes"],
                             item["additional_estimated_wire_bytes"],
                             item["unique_frame_payload_sizes"],
                             item["spearman_application_size_frame_count"],
                             scores["full"]["accuracy"], scores["size_only"]["accuracy"],
                             scores["frame_count_only"]["accuracy"],
                             scores["directional_count_only"]["accuracy"]))
    print("VERDICT", verdict, flush=True)


if __name__ == "__main__":
    main()
