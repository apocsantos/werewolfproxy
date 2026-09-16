#!/usr/bin/env python3
"""Model final TCP connection-volume targets over Stage13D public metadata.

No traffic is generated. Targets are optimistic end-total byte counts, not
an implementable authenticated cover-frame or timing policy.
"""

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
HERE = pathlib.Path(__file__).resolve().parent
SOURCE = ROOT / "tests" / "stage13_segmentation_study" / "sessions.csv"
sys.path.insert(0, str(ROOT / "tests" / "stage13_traffic_morphology"))
import analyze as baseline  # noqa: E402


def median(values):
    return baseline.median(values)


def percentile(values, q):
    return baseline.percentile(values, q)


def load_control():
    with SOURCE.open(newline="") as stream:
        original = list(csv.DictReader(stream))
    rows = []
    for row in original:
        up, down = int(row["total_c2s"]), int(row["total_s2c"])
        assert up > 0 and down > 0
        rows.append({"id": row["id"], "workload": row["workload"],
                     "sample": int(row["sample"]), "total_c2s": up,
                     "total_s2c": down, "total_wire": up + down,
                     "wire_up_fraction": up / (up + down)})
    counts = collections.Counter(row["workload"] for row in rows)
    assert len(rows) == 275
    assert counts == {name: 5 if name == "bulk_50mib" else 30
                      for name in baseline.WORKLOAD_CLASS}
    assert hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest() == (
        "c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17")
    with (HERE / "control_dataset.csv").open("w", newline="") as stream:
        fields = ("id", "workload", "sample", "total_c2s", "total_s2c", "total_wire")
        writer = csv.DictWriter(stream, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        writer.writerows({key: row[key] for key in fields} for row in rows)
    return rows, counts


def classify(rows, names):
    previous = baseline.FEATURES["tcp"]
    try:
        baseline.FEATURES["tcp"] = tuple(names)
        return baseline.classify(rows, "tcp")
    finally:
        baseline.FEATURES["tcp"] = previous


def score(rows, features):
    return {name: classify(rows, names) for name, names in features.items()}


def ceil_bucket(amount, policy):
    kind = policy["kind"]
    if kind in ("buckets", "random_upper"):
        return next((bucket for bucket in policy["buckets"] if bucket >= amount), amount)
    if kind == "power2":
        return 1 << (amount - 1).bit_length()
    if kind == "geometric":
        value = policy["anchor"]
        while value < amount:
            value = (value * policy["numerator"] + policy["denominator"] - 1) // policy["denominator"]
        return value
    if kind == "small_threshold":
        return policy["target"] if amount < policy["threshold"] else amount
    raise AssertionError(kind)


def target(amount, policy, rng, maximum):
    kind = policy["kind"]
    if kind == "constant_observed_max":
        return maximum
    if kind == "random_upper":
        buckets = policy["buckets"]
        first = next((i for i, bucket in enumerate(buckets) if bucket >= amount), None)
        if first is None:
            return amount
        probabilities = policy["probabilities_min_next_next2"]
        assert sum(probabilities) == 1
        draw = rng.random()
        offset = 0 if draw < probabilities[0] else (1 if draw < sum(probabilities[:2]) else 2)
        return buckets[min(first + offset, len(buckets) - 1)]
    return ceil_bucket(amount, policy)


def coarse_ratio_target(up, down, initial_total, policy, ratios):
    best = None
    for numerator, denominator in ratios:
        needed = max(initial_total, math.ceil(up * denominator / numerator),
                     math.ceil(down * denominator / (denominator - numerator)))
        candidate = ceil_bucket(needed, policy)
        # Integer rounding can leave a one-byte shortfall at a boundary.
        while candidate * numerator // denominator < up or (
                candidate - candidate * numerator // denominator < down):
            candidate = ceil_bucket(candidate + 1, policy)
        split = candidate * numerator // denominator
        choice = (candidate, abs(split / candidate - up / (up + down)), split)
        if best is None or choice < best:
            best = choice
    return best[2], best[0] - best[2]


def model(control, policy, mode, rng, maxima, ratios):
    rows = []
    for row in control:
        up, down, current = row["total_c2s"], row["total_s2c"], row["total_wire"]
        if mode == "current":
            new_up, new_down = up, down
        elif mode == "directional":
            new_up = target(up, policy, rng, maxima["c2s"])
            new_down = target(down, policy, rng, maxima["s2c"])
        else:
            total = target(current, policy, rng, maxima["total"])
            if mode == "total_proportional":
                new_up = total * up // current
                new_down = total - new_up
            elif mode == "total_ratio":
                new_up, new_down = coarse_ratio_target(up, down, total, policy, ratios)
            else:
                raise AssertionError(mode)
        assert new_up >= up and new_down >= down
        rows.append({"id": row["id"], "workload": row["workload"],
                     "sample": row["sample"], "total_c2s": new_up,
                     "total_s2c": new_down, "total_wire": new_up + new_down,
                     "wire_up_fraction": new_up / (new_up + new_down),
                     "added": new_up + new_down - current,
                     "amplification": (new_up + new_down) / current})
    return rows


def workload_overlap(rows):
    distributions = {}
    for name in baseline.WORKLOAD_CLASS:
        values = [row["total_wire"] for row in rows if row["workload"] == name]
        distributions[name] = collections.Counter(values)
    overlap = {}
    names = list(distributions)
    for i, left in enumerate(names):
        for right in names[i + 1:]:
            a, b = distributions[left], distributions[right]
            overlap[f"{left}|{right}"] = round(sum(
                min(a[key] / sum(a.values()), b[key] / sum(b.values()))
                for key in a.keys() | b.keys()), 4)
    return {"pairwise_exact_total_histogram_overlap": overlap,
            "per_workload_top_three_totals": {
                name: counts.most_common(3) for name, counts in distributions.items()}}


def metrics(rows, control, policy, mode):
    previous = {row["id"]: row for row in control}
    added = [row["added"] for row in rows]
    amp = [row["amplification"] for row in rows]
    percent = [100 * row["added"] / previous[row["id"]]["total_wire"] for row in rows]
    by_workload = {}
    for workload in baseline.WORKLOAD_CLASS:
        group = [row for row in rows if row["workload"] == workload]
        group_added = [r["added"] for r in group]
        group_amplification = [r["amplification"] for r in group]
        by_workload[workload] = {
            "n": len(group),
            "median_added_bytes": median(group_added),
            "p90_added_bytes": percentile(group_added, .9),
            "p95_added_bytes": percentile(group_added, .95),
            "p99_added_bytes": percentile(group_added, .99),
            "max_added_bytes": max(group_added),
            "median_amplification": median(group_amplification),
            "p95_amplification": percentile(group_amplification, .95),
            "max_amplification": max(group_amplification),
            "median_c2s_bytes": median([r["total_c2s"] for r in group]),
            "median_s2c_bytes": median([r["total_s2c"] for r in group]),
            "median_direction_ratio": median([r["wire_up_fraction"] for r in group]),
        }
    return {
        "policy": policy["name"], "mode": mode,
        "implementability": ("ONLINE_INCREMENTAL" if policy["kind"] == "current" else
                             "NOT_REALISTIC" if policy["kind"] == "constant_observed_max" else
                             "CLOSE_TIME_ONLY"),
        "median_added_bytes": median(added),
        "p90_added_bytes": percentile(added, .9),
        "p95_added_bytes": percentile(added, .95),
        "p99_added_bytes": percentile(added, .99),
        "max_added_bytes": max(added),
        "median_added_percent": median(percent),
        "median_amplification": median(amp),
        "p95_amplification": percentile(amp, .95),
        "max_observed_amplification": max(amp),
        "total_dataset_added_bytes": sum(added),
        "unique_final_total_sizes": len({r["total_wire"] for r in rows}),
        "unique_final_direction_pairs": len({(r["total_c2s"], r["total_s2c"]) for r in rows}),
        "by_workload": by_workload,
        "overlap": workload_overlap(rows),
    }


def bucket_edge_amplification(policy, min_total):
    kind = policy["kind"]
    if kind in ("buckets", "random_upper"):
        buckets = policy["buckets"]
        comparisons = []
        for i, boundary in enumerate(buckets[:-1]):
            if boundary + 1 < min_total:
                continue
            step = 3 if kind == "random_upper" else 1
            upper = buckets[min(i + step, len(buckets) - 1)]
            comparisons.append((upper / (boundary + 1), boundary + 1, upper))
        return max(comparisons, default=(1, min_total, min_total))
    if kind == "power2":
        return (2.0, "just above power-of-two boundary", "next power of two")
    if kind == "geometric":
        return (policy["numerator"] / policy["denominator"],
                "just above geometric boundary", "next geometric bucket")
    if kind == "small_threshold":
        return (policy["target"] / min_total, min_total, policy["target"])
    if kind == "constant_observed_max":
        return (None, "smallest observed connection", "dataset maximum")
    return (1, None, None)


def pareto(metrics_by_name, scores):
    rows = []
    for name, item in metrics_by_name.items():
        rows.append((name, item["median_added_percent"],
                     scores[name]["total_only"]["accuracy"],
                     scores[name]["full_volume"]["accuracy"]))
    frontier = []
    for name, cost, accuracy, full in rows:
        dominated = any((other_cost <= cost and other_acc <= accuracy and
                         (other_cost < cost or other_acc < accuracy))
                        for other_name, other_cost, other_acc, _ in rows
                        if other_name != name)
        if not dominated:
            frontier.append({"candidate": name, "median_added_percent": cost,
                             "total_accuracy": accuracy, "full_accuracy": full})
    return sorted(frontier, key=lambda row: (row["median_added_percent"],
                                              row["total_accuracy"], row["candidate"]))


def main():
    policy_bytes = (HERE / "policies.json").read_bytes()
    config = json.loads(policy_bytes)
    control, counts = load_control()
    maxima = {"c2s": max(r["total_c2s"] for r in control),
              "s2c": max(r["total_s2c"] for r in control),
              "total": max(r["total_wire"] for r in control)}
    ratios = config["fixed_ratio_targets"]
    first = config["candidates"][0]
    assert first["name"] == "A_current" and first["modes"] == ["current"]
    control_replay = model(control, first, "current", random.Random(13), maxima, ratios)
    assert all(row["added"] == 0 and row["total_c2s"] == original["total_c2s"] and
               row["total_s2c"] == original["total_s2c"]
               for row, original in zip(control_replay, control))
    control_scores = score(control_replay, config["observer_features"])
    assert control_scores["full_volume"]["majority_test_baseline"] == .3494
    assert abs(control_scores["total_only"]["accuracy"] - .747) <= .02
    assert abs(control_scores["directional_totals"]["accuracy"] - .5301) <= .02
    assert abs(control_scores["ratio_only"]["accuracy"] - .3855) <= .02
    results = {"A_current": metrics(control_replay, control, first, "current")}
    classifiers = {"A_current": control_scores}
    print("CONTROL_VALIDATED", len(control), control_scores["total_only"]["accuracy"], flush=True)
    for policy in config["candidates"][1:]:
        for mode in policy["modes"]:
            name = policy["name"] if mode == "current" else policy["name"] + "__" + mode
            rng = random.Random(config["simulation_seed"])
            rows = model(control, policy, mode, rng, maxima, ratios)
            results[name] = metrics(rows, control, policy, mode)
            classifiers[name] = score(rows, config["observer_features"])
            print(name, classifiers[name]["total_only"]["accuracy"],
                  results[name]["median_added_percent"], flush=True)
    a = classifiers["A_current"]
    budgets = {}
    for budget in config["budgets_median_percent"]:
        eligible = [name for name, item in results.items()
                    if item["median_added_percent"] <= budget]
        best = min(eligible, key=lambda name: (
            classifiers[name]["total_only"]["accuracy"],
            classifiers[name]["full_volume"]["accuracy"],
            results[name]["median_added_percent"], name))
        budgets[str(budget)] = {"best_raw_candidate": best,
                               "total_accuracy": classifiers[best]["total_only"]["accuracy"],
                               "full_accuracy": classifiers[best]["full_volume"]["accuracy"],
                               "median_added_percent": results[best]["median_added_percent"],
                               "eligible_count": len(eligible),
                               "production_recommendation": False}
    f1 = classifiers["F_constant__total_proportional"]
    f2 = classifiers["F_constant__directional"]
    assert results["F_constant__total_proportional"]["unique_final_total_sizes"] == 1
    assert results["F_constant__directional"]["unique_final_direction_pairs"] == 1
    control_total = a["total_only"]["accuracy"]
    threshold = config["decision_rule"]["minimum_absolute_accuracy_reduction_points"] / 100
    cheap = [name for name, item in results.items() if name != "A_current" and
             item["median_added_percent"] <= config["decision_rule"]["maximum_median_added_percent"] and
             (control_total - classifiers[name]["total_only"]["accuracy"] >= threshold or
              a["full_volume"]["accuracy"] - classifiers[name]["full_volume"]["accuracy"] >= threshold)]
    theoretical_floor = f2["full_volume"]["accuracy"]
    majority = a["full_volume"]["majority_test_baseline"]
    assert theoretical_floor == majority
    if theoretical_floor > majority + .10:
        verdict = "VOLUME_ONLY_INSUFFICIENT"
    elif cheap:
        # All candidate end totals remain close-time-only and need a reviewed
        # authenticated cover protocol plus timing design before deployment.
        verdict = "NO_ACCEPTABLE_VOLUME_POLICY"
    else:
        verdict = "VOLUME_NORMALIZATION_EFFECTIVE_BUT_TOO_EXPENSIVE"
    amplification = {policy["name"]: {
        "theoretical_bucket_edge_total_mode": bucket_edge_amplification(
            policy, min(r["total_wire"] for r in control)),
        "max_observed_amplification_all_modes": max(
            results[policy["name"] if mode == "current" else policy["name"] + "__" + mode]
            ["max_observed_amplification"] for mode in policy["modes"]),
    } for policy in config["candidates"]}
    summary = {
        "schema": 1,
        "source_head": "3a27640835ce2d596163b7d7f65875b10db25b91",
        "source_dataset": str(SOURCE.relative_to(ROOT)),
        "source_dataset_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        "policy_sha256": hashlib.sha256(policy_bytes).hexdigest(),
        "control": {"connections": len(control), "workloads": counts,
                    "directional_maximum_wire_bytes": maxima,
                    "exact_replay": True, "majority_baseline": majority,
                    "total_only_accuracy": control_total,
                    "directional_accuracy": a["directional_totals"]["accuracy"],
                    "ratio_accuracy": a["ratio_only"]["accuracy"]},
        "candidate_metrics": results,
        "budget_frontier": budgets,
        "pareto_frontier": pareto(results, classifiers),
        "theoretical_volume_floor": {"F1_total_only_accuracy": f1["total_only"]["accuracy"],
                                     "F1_full_accuracy": f1["full_volume"]["accuracy"],
                                     "F2_total_only_accuracy": f2["total_only"]["accuracy"],
                                     "F2_full_accuracy": theoretical_floor},
        "online_realistic_pre_tail_floor": {
            "total_only_accuracy": control_total,
            "directional_accuracy": a["directional_totals"]["accuracy"],
            "ratio_accuracy": a["ratio_only"]["accuracy"],
            "reason": "all modeled padding is appended after original traffic; pre-tail bytes unchanged"},
        "cheap_candidates_meeting_classifier_gate": cheap,
        "verdict": verdict,
        "recommended_next_stage": ("stop automatic volume morphing; consider opt-in privacy profile cost study"
                                   if verdict == "VOLUME_NORMALIZATION_EFFECTIVE_BUT_TOO_EXPENSIVE"
                                   else "stop volume normalization work"),
    }
    write_json("summary.json", summary)
    write_json("classifier_results.json", classifiers)
    write_json("pareto_frontier.json", {"budgets": budgets,
                                        "nondominated_raw_total_accuracy": summary["pareto_frontier"]})
    write_json("amplification.json", amplification)
    with (HERE / "candidate_metrics.csv").open("w", newline="") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(("candidate", "mode", "implementability", "median_added_percent",
                         "median_added_bytes", "p95_added_bytes", "max_added_bytes",
                         "total_dataset_added_bytes", "median_amplification",
                         "max_observed_amplification", "full_accuracy", "total_accuracy",
                         "directional_accuracy", "ratio_accuracy"))
        for name, item in results.items():
            scores = classifiers[name]
            writer.writerow((name, item["mode"], item["implementability"],
                             item["median_added_percent"], item["median_added_bytes"],
                             item["p95_added_bytes"], item["max_added_bytes"],
                             item["total_dataset_added_bytes"], item["median_amplification"],
                             item["max_observed_amplification"],
                             *(scores[key]["accuracy"] for key in
                               ("full_volume", "total_only", "directional_totals", "ratio_only"))))
    print("VERDICT", verdict, flush=True)


def write_json(name, value):
    (HERE / name).write_text(json.dumps(value, indent=2) + "\n")


if __name__ == "__main__":
    main()
