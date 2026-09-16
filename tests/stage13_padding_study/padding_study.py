#!/usr/bin/env python3
"""Replay measured Stage13B TCP forwarding frames through fixed padding policies."""

import collections
import csv
import hashlib
import json
import math
import pathlib
import random
import statistics
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests" / "stage13_traffic_morphology"))
import analyze as baseline  # noqa: E402

HERE = pathlib.Path(__file__).resolve().parent
FRAME_FIELDS_INT = (
    "payload_bytes", "padded_plaintext_bytes", "ciphertext_bytes",
    "inner_submitted_bytes", "outer_tls_record_bytes", "final_frame", "frame_count",
)
SESSION_FIELDS_INT = (
    "sample", "total_c2s", "total_s2c", "tls_record_count",
    "frame_count_c2s", "frame_count_s2c", "frame_wire_c2s", "frame_wire_s2c",
    "nonforward_tls_c2s", "nonforward_tls_s2c",
    "tls_public_handshake_bytes", "tls_encrypted_handshake_bytes",
    "stage10_handshake_bytes", "close_record_bytes",
)
SESSION_FIELDS_FLOAT = ("duration_ms", "time_to_first_response_ms")
ONE_WAY_MESSAGES = {
    "tiny": 2, "interactive": 16, "request_1kib": 2,
    "transfer_64kib": 2, "transfer_1mib": 2,
    "bulk_50mib": 2, "asymmetric_upload": 2, "asymmetric_download": 2,
}


def read_csv(path, ints, floats=()):
    with path.open(newline="") as stream:
        rows = list(csv.DictReader(stream))
    for row in rows:
        for name in ints:
            row[name] = int(row[name])
        for name in floats:
            row[name] = float(row[name]) if row[name] else None
    return rows


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def allowed_current(payload, padded):
    minimum = max(payload + 2, 768)
    return payload <= 2046 and minimum <= padded <= 2048 and (
        padded - minimum) % 128 == 0


def smallest_bucket(buckets, payload):
    return next(size for size in buckets if size >= payload + 2)


def choose(policy, frame, rng):
    kind = policy["kind"]
    payload = frame["payload_bytes"]
    if kind == "observed_current":
        return frame["padded_plaintext_bytes"]
    if kind == "smallest_bucket":
        return smallest_bucket(policy["buckets"], payload)
    if kind == "random_upper":
        buckets = policy["buckets"]
        first = next(i for i, size in enumerate(buckets) if size >= payload + 2)
        r = rng.random()
        minimum, next_one, maximum = policy["probabilities_min_next_max"]
        assert abs(minimum + next_one + maximum - 1) < 1e-9
        index = first if r < minimum else min(first + 1, len(buckets) - 1)
        if r >= minimum + next_one:
            index = len(buckets) - 1
        return buckets[index]
    if kind == "small_flatten":
        if payload > policy["threshold_payload_bytes"]:
            return frame["padded_plaintext_bytes"]
        return (policy["small_buckets"][0] if rng.random() <
                policy["small_probabilities"][0] else policy["small_buckets"][1])
    if kind == "constant_max":
        return policy["padded_plaintext_bytes"]
    raise AssertionError(f"unknown candidate {kind}")


def fresh_state(sessions):
    return {row["id"]: {
        "frame_wire_c2s": 0, "frame_wire_s2c": 0,
        "inner_bytes": 0, "final_frame_wire_c2s": 0,
        "final_frame_wire_s2c": 0,
    } for row in sessions}


def simulate(name, policy, frames, sessions, seed):
    rng = random.Random(seed)
    state = fresh_state(sessions)
    sizes = collections.Counter()
    by_workload_sizes = collections.defaultdict(collections.Counter)
    for frame in frames:
        count = frame["frame_count"]
        # Deterministic candidates can be multiplied by count. Randomized
        # candidates draw once per original frame, without altering the
        # frame count or ordering-dependent classifier features.
        outcomes = collections.Counter(
            choose(policy, frame, rng) for _ in range(count))
        for padded, occurrences in outcomes.items():
            assert frame["payload_bytes"] + 2 <= padded <= 2048
            wire = padded + 42
            sizes[wire] += occurrences
            by_workload_sizes[frame["workload"]][wire] += occurrences
            item = state[frame["connection_id"]]
            direction = frame["direction"]
            item[f"frame_wire_{direction}"] += wire * occurrences
            item["inner_bytes"] += (padded + 20) * occurrences
            if frame["final_frame"]:
                assert count == 1 and occurrences == 1
                item[f"final_frame_wire_{direction}"] = wire
    rows = []
    for session in sessions:
        item = state[session["id"]]
        c2s = session["nonforward_tls_c2s"] + item["frame_wire_c2s"]
        s2c = session["nonforward_tls_s2c"] + item["frame_wire_s2c"]
        row = {
            "id": session["id"], "workload": session["workload"],
            "sample": session["sample"], "total_c2s": c2s, "total_s2c": s2c,
            "tls_record_count": session["tls_record_count"],
            "duration_ms": session["duration_ms"],
            "frame_count_c2s": session["frame_count_c2s"],
            "frame_count_s2c": session["frame_count_s2c"],
            "frame_count_total": session["frame_count_c2s"] + session["frame_count_s2c"],
            "frame_wire_c2s": item["frame_wire_c2s"],
            "frame_wire_s2c": item["frame_wire_s2c"],
            "final_frame_wire_c2s": item["final_frame_wire_c2s"],
            "final_frame_wire_s2c": item["final_frame_wire_s2c"],
            "inner_bytes": item["inner_bytes"],
        }
        if name == "A_current":
            assert (c2s, s2c) == (session["total_c2s"], session["total_s2c"])
            assert (item["frame_wire_c2s"], item["frame_wire_s2c"]) == (
                session["frame_wire_c2s"], session["frame_wire_s2c"])
        rows.append(row)
    return rows, sizes, by_workload_sizes


def classify_with_features(rows, names):
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


def entropy(counter):
    total = sum(counter.values())
    return round(-sum((n / total) * math.log2(n / total) for n in counter.values()), 4)


def worst_case_amplification(policy):
    # Ratio to the smallest currently possible wire record for this payload.
    # Randomized policies use the largest reachable bucket; E preserves the
    # current upper bound for larger frames.
    worst_ratio = 0
    worst_payload = 0
    for payload in range(1, 2047):
        kind = policy["kind"]
        if kind in ("observed_current", "constant_max"):
            maximum = 2048
        elif kind in ("smallest_bucket", "random_upper"):
            maximum = (smallest_bucket(policy["buckets"], payload)
                       if kind == "smallest_bucket" else policy["buckets"][-1])
        else:
            maximum = 2048
        ratio = (maximum + 42) / (max(payload + 2, 768) + 42)
        if ratio > worst_ratio:
            worst_ratio, worst_payload = ratio, payload
    return {"max_ratio_to_current_minimum_record": round(worst_ratio, 3),
            "payload_bytes_at_max": worst_payload,
            "one_byte_payload_max_wire_bytes": (
                2048 + 42 if policy["kind"] not in ("smallest_bucket",) else
                smallest_bucket(policy["buckets"], 1) + 42)}


def candidate_metrics(name, policy, rows, sizes, workload_sizes, current, frames):
    current_by_id = {row["id"]: row for row in current}
    sized = [row for row in rows if sum(baseline.APP_BYTES[row["workload"]])]
    application_size = [sum(baseline.APP_BYTES[row["workload"]]) for row in sized]
    workload_metrics = {}
    all_overhead = []
    for workload in baseline.WORKLOAD_CLASS:
        group = [row for row in rows if row["workload"] == workload]
        added = [row["total_c2s"] + row["total_s2c"] -
                 current_by_id[row["id"]]["total_c2s"] -
                 current_by_id[row["id"]]["total_s2c"] for row in group]
        overhead = [100 * delta / (current_by_id[row["id"]]["total_c2s"] +
                                   current_by_id[row["id"]]["total_s2c"])
                    for row, delta in zip(group, added)]
        all_overhead.extend(overhead)
        app_bytes = sum(baseline.APP_BYTES[workload])
        workload_metrics[workload] = {
            "connections": len(group),
            "median_added_wire_bytes_per_connection": median(added),
            "median_added_wire_bytes_per_one_way_message": (
                median([value / ONE_WAY_MESSAGES[workload] for value in added])
                if workload in ONE_WAY_MESSAGES else None),
            "median_additional_wire_percent": median(overhead),
            "p90_additional_wire_percent": percentile(overhead, .9),
            "median_wire_to_application_ratio": (
                median([(row["total_c2s"] + row["total_s2c"]) / app_bytes
                        for row in group]) if app_bytes else None),
            "median_total_wire_bytes": median([
                row["total_c2s"] + row["total_s2c"] for row in group]),
            "median_frame_count": median([row["frame_count_total"] for row in group]),
            "unique_forwarding_record_sizes": len(workload_sizes[workload]),
            "forwarding_record_size_top_five": workload_sizes[workload].most_common(5),
        }
    total_frames = sum(sizes.values())
    total_forwarding_bytes = sum(size * count for size, count in sizes.items())
    forwarding_rows = [dict(row, total_c2s=row["frame_wire_c2s"],
                            total_s2c=row["frame_wire_s2c"],
                            tls_record_count=row["frame_count_total"]) for row in rows]
    return {
        "definition": policy,
        "total_inner_submitted_bytes": sum(row["inner_bytes"] for row in rows),
        "total_forwarding_outer_tls_bytes": total_forwarding_bytes,
        "total_outer_estimated_bytes": sum(row["total_c2s"] + row["total_s2c"] for row in rows),
        "frame_count": total_frames,
        "unique_observable_forwarding_frame_sizes": len(sizes),
        "forwarding_frame_size_entropy_bits": entropy(sizes),
        "largest_size_share_percent": round(100 * max(sizes.values()) / total_frames, 3),
        "forwarding_record_size_top_ten": sizes.most_common(10),
        "spearman_application_size_vs_wire_estimate": baseline.spearman(
            application_size, [row["total_c2s"] + row["total_s2c"] for row in sized]),
        "spearman_application_size_vs_frame_count": baseline.spearman(
            application_size, [row["frame_count_total"] for row in sized]),
        "classifier_full": baseline.classify(rows, "tcp"),
        "classifier_size_only": baseline.classify(rows, "tcp", size_only=True),
        "classifier_forwarding_only_full": baseline.classify(
            forwarding_rows, "tcp"),
        "classifier_forwarding_only_size_only": baseline.classify(
            forwarding_rows, "tcp", size_only=True),
        "classifier_directional_bytes_only": classify_with_features(
            rows, ("frame_wire_c2s", "frame_wire_s2c")),
        "classifier_final_frame_sizes_only": classify_with_features(
            rows, ("final_frame_wire_c2s", "final_frame_wire_s2c")),
        "overhead_percent_all_connections": {
            "p50": percentile(all_overhead, .5),
            "p90": percentile(all_overhead, .9),
            "p95": percentile(all_overhead, .95),
            "p99": percentile(all_overhead, .99),
        },
        "max_median_additional_wire_percent_nonempty_workloads": max(
            workload_metrics[name]["median_additional_wire_percent"]
            for name in ONE_WAY_MESSAGES),
        "worst_case_amplification": worst_case_amplification(policy),
        "workloads": workload_metrics,
    }


def validate_model(frames, sessions, manifest, config, current_metrics, current_rows):
    assert len(sessions) == manifest["connections"] == 275
    assert sum(frame["frame_count"] for frame in frames) == manifest["frames"]
    by_connection = collections.Counter()
    dominant = collections.Counter()
    payload_histogram = collections.Counter()
    for frame in frames:
        n = frame["frame_count"]
        assert allowed_current(frame["payload_bytes"], frame["padded_plaintext_bytes"])
        assert frame["ciphertext_bytes"] == frame["padded_plaintext_bytes"] + 16
        assert frame["inner_submitted_bytes"] == frame["padded_plaintext_bytes"] + 20
        assert frame["outer_tls_record_bytes"] == frame["padded_plaintext_bytes"] + 42
        assert frame["final_frame"] in (0, 1)
        assert not frame["final_frame"] or n == 1
        by_connection[(frame["connection_id"], frame["direction"])] += n
        payload_histogram[frame["payload_bytes"]] += n
        if frame["payload_bytes"] == 1400:
            dominant[frame["padded_plaintext_bytes"]] += n
    for session in sessions:
        assert by_connection[(session["id"], "c2s")] == session["frame_count_c2s"]
        assert by_connection[(session["id"], "s2c")] == session["frame_count_s2c"]
    expected = {1402 + 128 * i for i in range(6)}
    assert set(dominant) == expected
    dominant_count = sum(dominant.values())
    variation = 0.5 * sum(abs(dominant[size] / dominant_count - 1 / 6)
                          for size in expected)
    assert variation <= config["model_validation"][
        "dominant_payload_1400_uniform_total_variation_max"]
    stage13b = json.loads((ROOT / "tests/stage13_tcp_coalescing/summary.json").read_text())[
        "transports"]["tcp"]
    fresh_sized = [r for r in current_rows if sum(baseline.APP_BYTES[r["workload"]])]
    fresh_app_size = [sum(baseline.APP_BYTES[r["workload"]]) for r in fresh_sized]
    fresh_corr = baseline.spearman(fresh_app_size, [
        r["total_c2s"] + r["total_s2c"] for r in fresh_sized])
    assert abs(fresh_corr - stage13b["size_leakage_spearman"]["total_wire_bytes"]) <= (
        config["model_validation"]["fresh_vs_stage13b_spearman_difference_max"])
    assert abs(current_metrics["classifier_full"]["accuracy"] -
               stage13b["classifier"]["accuracy"]) <= config[
                   "model_validation"]["fresh_vs_stage13b_classifier_accuracy_difference_max"]
    assert abs(current_metrics["classifier_size_only"]["accuracy"] -
               stage13b["classifier_size_only"]["accuracy"]) <= config[
                   "model_validation"]["fresh_vs_stage13b_classifier_accuracy_difference_max"]
    assert abs(current_metrics["workloads"]["bulk_50mib"][
        "median_wire_to_application_ratio"] - stage13b["workloads"]["bulk_50mib"][
            "median_wire_to_app_bytes_ratio"]) <= config[
                "model_validation"]["fresh_vs_stage13b_50mib_ratio_difference_max"]
    return {
        "all_frames_match_production_payload_padding_formula": True,
        "all_ciphertext_inner_and_tls_lengths_match": True,
        "all_275_connections_reconstruct_exact_observed_wire_bytes": True,
        "dominant_payload_1400_frames": dominant_count,
        "dominant_payload_1400_bucket_counts": dict(sorted(dominant.items())),
        "dominant_payload_1400_uniform_total_variation": round(variation, 5),
        "payload_length_top_twenty": payload_histogram.most_common(20),
        "payload_length_unique_count": len(payload_histogram),
        "payload_at_most_256_frames": sum(n for size, n in payload_histogram.items()
                                          if size <= 256),
        "stage13b_observed_full_accuracy": stage13b["classifier"]["accuracy"],
        "stage13b_observed_size_only_accuracy": stage13b["classifier_size_only"]["accuracy"],
        "stage13b_observed_wire_spearman": stage13b[
            "size_leakage_spearman"]["total_wire_bytes"],
        "fresh_current_wire_spearman": fresh_corr,
    }


def budget_frontier(candidates, config):
    output = {}
    for limit in config["budget_levels_percent"]:
        feasible = [(name, part) for name, part in candidates.items()
                    if part["max_median_additional_wire_percent_nonempty_workloads"] <= limit]
        winner, part = min(feasible, key=lambda item: (
            item[1]["classifier_size_only"]["accuracy"],
            item[1]["classifier_full"]["accuracy"],
            item[1]["max_median_additional_wire_percent_nonempty_workloads"]))
        output[str(limit)] = {
            "best_by_frozen_classifier_score": winner,
            "size_only_accuracy": part["classifier_size_only"]["accuracy"],
            "full_accuracy": part["classifier_full"]["accuracy"],
            "max_workload_median_additional_wire_percent": part[
                "max_median_additional_wire_percent_nonempty_workloads"],
            "eligible_candidates": [name for name, _ in feasible],
            "warning": "Classifier score alone does not establish protocol fingerprint safety",
        }
    return output


def main():
    config_path = HERE / "policies.json"
    config = json.loads(config_path.read_text())
    assert config["frozen_before_classifier_evaluation"]
    frames = read_csv(HERE / "baseline_frames.csv", FRAME_FIELDS_INT)
    sessions = read_csv(HERE / "baseline_sessions.csv", SESSION_FIELDS_INT,
                        SESSION_FIELDS_FLOAT)
    manifest = json.loads((HERE / "capture_manifest.json").read_text())
    assert manifest["source_head"] == "af0a02dea2f11f157eaf81080acb869e15e11426"
    assert manifest["lock_sha256"] == config.get(
        "lock_sha256", manifest["lock_sha256"])
    candidates = {}
    rows_by_candidate = {}
    control = config["candidates"]["A_current"]
    control_rows, control_sizes, control_workloads = simulate(
        "A_current", control, frames, sessions, config["simulation_seed"])
    rows_by_candidate["A_current"] = control_rows
    candidates["A_current"] = candidate_metrics(
        "A_current", control, control_rows, control_sizes,
        control_workloads, control_rows, frames)
    validation = validate_model(
        frames, sessions, manifest, config, candidates["A_current"],
        control_rows)
    # No candidate is evaluated until the control has exactly reconstructed
    # all measured forwarding and nonforwarding wire bytes.
    for name, policy in config["candidates"].items():
        if name == "A_current":
            continue
        rows, sizes, workload_sizes = simulate(
            name, policy, frames, sessions, config["simulation_seed"])
        rows_by_candidate[name] = rows
        candidates[name] = candidate_metrics(
            name, policy, rows, sizes, workload_sizes, control_rows, frames)
    counts_only = classify_with_features(
        rows_by_candidate["A_current"], ("frame_count_total",))
    directional_counts = classify_with_features(
        rows_by_candidate["A_current"],
        ("frame_count_c2s", "frame_count_s2c"))
    final_frames = [frame for frame in frames if frame["final_frame"]]
    final_ablation_policy = {
        "kind": "observed_current",
    }
    ablated = []
    for frame in frames:
        changed = frame.copy()
        if changed["final_frame"]:
            changed["padded_plaintext_bytes"] = 2048
        ablated.append(changed)
    final_rows, _, _ = simulate(
        "final_ablation", final_ablation_policy, ablated, sessions,
        config["simulation_seed"])
    final_ablation = baseline.classify(final_rows, "tcp", size_only=True)
    final_hist = collections.Counter()
    for frame in final_frames:
        final_hist[frame["outer_tls_record_bytes"]] += frame["frame_count"]
    final_by_workload = {
        workload: {
            "count": sum(frame["frame_count"] for frame in final_frames
                         if frame["workload"] == workload),
            "partial_below_1400_count": sum(frame["frame_count"] for frame in final_frames
                                            if frame["workload"] == workload and
                                            frame["payload_bytes"] < 1400),
            "payload_bytes_top_five": collections.Counter({
                size: sum(frame["frame_count"] for frame in final_frames
                          if frame["workload"] == workload and
                          frame["payload_bytes"] == size)
                for size in {frame["payload_bytes"] for frame in final_frames
                             if frame["workload"] == workload}
            }).most_common(5),
        } for workload in baseline.WORKLOAD_CLASS
    }
    budgets = budget_frontier(candidates, config)
    current = candidates["A_current"]
    constant = candidates["F_constant_max"]
    majority = current["classifier_full"]["majority_test_baseline"]
    floor = config["padding_only_floor_above_majority_percentage_points"] / 100
    if (constant["classifier_size_only"]["accuracy"] >= majority + floor and
            directional_counts["accuracy"] >= majority + floor):
        verdict = "PADDING_ONLY_INSUFFICIENT"
        next_stage = "Stage13D frame segmentation morphology study"
    else:
        material = config["decision_materiality_percentage_points"] / 100
        viable = [name for name, part in candidates.items() if name != "A_current"
                  and (current["classifier_size_only"]["accuracy"] -
                       part["classifier_size_only"]["accuracy"] >= material or
                       current["classifier_full"]["accuracy"] -
                       part["classifier_full"]["accuracy"] >= material)
                  and part["workloads"]["bulk_50mib"][
                      "median_additional_wire_percent"] <= config[
                          "acceptable_bulk_median_additional_wire_percent"]
                  and part["largest_size_share_percent"] <= current[
                      "largest_size_share_percent"]]
        verdict = ("PADDING_POLICY_CANDIDATE_FOUND" if viable else
                   "NO_ACCEPTABLE_PADDING_POLICY")
        next_stage = ("Stage13D controlled production padding experiment" if viable else
                      "Stop padding work; review residual segmentation evidence")
    config_hash = hashlib.sha256(config_path.read_bytes()).hexdigest()
    summary = {
        "schema": 1, "policy_definition_sha256": config_hash,
        "dataset": manifest, "model_validation": validation,
        "frame_count_only_classifier": counts_only,
        "directional_frame_count_classifier": directional_counts,
        "final_frames": {
            "count": sum(frame["frame_count"] for frame in final_frames),
            "partial_below_1400_count": sum(frame["frame_count"] for frame in final_frames
                                            if frame["payload_bytes"] < 1400),
            "record_size_top_ten": final_hist.most_common(10),
            "by_workload": final_by_workload,
            "current_size_only_accuracy": current["classifier_size_only"]["accuracy"],
            "flatten_only_final_frame_size_only_accuracy": final_ablation["accuracy"],
        },
        "candidates": candidates, "budget_frontier": budgets,
        "majority_baseline": majority,
        "verdict": verdict, "recommended_next_stage": next_stage,
    }
    write_json(HERE / "summary.json", summary)
    write_json(HERE / "classifier_results.json", {
        "majority_baseline": majority,
        "frame_count_only": counts_only,
        "directional_frame_counts": directional_counts,
        "candidate_results": {name: {
            "full": part["classifier_full"],
            "size_only": part["classifier_size_only"],
            "directional_bytes_only": part["classifier_directional_bytes_only"],
            "final_frame_sizes_only": part["classifier_final_frame_sizes_only"],
            "forwarding_only_full": part["classifier_forwarding_only_full"],
            "forwarding_only_size_only": part["classifier_forwarding_only_size_only"],
        } for name, part in candidates.items()},
    })
    write_json(HERE / "overhead_budget.json", budgets)
    with (HERE / "candidate_metrics.csv").open("w", newline="") as stream:
        fields = ("candidate", "unique_record_sizes", "frame_size_entropy_bits",
                  "largest_size_share_percent", "wire_spearman",
                  "frame_count_spearman", "full_accuracy", "size_only_accuracy",
                  "total_inner_bytes", "total_outer_estimated_bytes",
                  "max_workload_median_overhead_percent", "bulk_median_overhead_percent")
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(fields)
        for name, part in candidates.items():
            writer.writerow((name, part["unique_observable_forwarding_frame_sizes"],
                             part["forwarding_frame_size_entropy_bits"],
                             part["largest_size_share_percent"],
                             part["spearman_application_size_vs_wire_estimate"],
                             part["spearman_application_size_vs_frame_count"],
                             part["classifier_full"]["accuracy"],
                             part["classifier_size_only"]["accuracy"],
                             part["total_inner_submitted_bytes"],
                             part["total_outer_estimated_bytes"],
                             part["max_median_additional_wire_percent_nonempty_workloads"],
                             part["workloads"]["bulk_50mib"][
                                 "median_additional_wire_percent"]))
    print("MODEL_VALIDATED", manifest["frames"], "frames across", len(sessions),
          "connections", flush=True)
    for name, part in candidates.items():
        print(name, "full", part["classifier_full"]["accuracy"], "size_only",
              part["classifier_size_only"]["accuracy"], "bulk_added_percent",
              part["workloads"]["bulk_50mib"]["median_additional_wire_percent"], flush=True)
    print("VERDICT", verdict, flush=True)


if __name__ == "__main__":
    main()
