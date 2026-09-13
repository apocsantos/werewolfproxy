#!/usr/bin/env python3
"""Summarize metadata-only production morphology captures without ML packages."""

import argparse
import collections
import csv
import hashlib
import json
import math
import pathlib
import random
import statistics

WORKLOAD_CLASS = {
    "handshake_only": "idle_or_empty", "idle": "idle_or_empty",
    "tiny": "tiny_interactive", "interactive": "tiny_interactive",
    "request_1kib": "medium", "transfer_64kib": "medium",
    "transfer_1mib": "bulk_or_asymmetric", "bulk_50mib": "bulk_or_asymmetric",
    "asymmetric_upload": "bulk_or_asymmetric",
    "asymmetric_download": "bulk_or_asymmetric",
}
APP_BYTES = {
    "handshake_only": (0, 0), "idle": (0, 0),
    "tiny": (32, 32), "interactive": (704, 704),
    "request_1kib": (1024, 1024),
    "transfer_64kib": (65536, 65536),
    "transfer_1mib": (1048576, 1048576),
    "bulk_50mib": (32, 50 * 1048576),
    "asymmetric_upload": (1048576, 32),
    "asymmetric_download": (32, 4 * 1048576),
}
FEATURES = {
    # A TCP recv() boundary is an artifact of the transparent relay, not an
    # on-path packet boundary. Keep this classifier to byte totals and public
    # TLS record count plus duration.
    "tcp": ("total_c2s", "total_s2c", "tls_record_count", "duration_ms"),
    "quic": ("total_c2s", "total_s2c", "count_c2s", "count_s2c",
             "duration_ms", "largest_unit", "burst_count", "direction_changes"),
}
SIZE_ONLY_FEATURES = {
    "tcp": ("total_c2s", "total_s2c", "tls_record_count"),
    "quic": ("total_c2s", "total_s2c", "count_c2s", "count_s2c",
             "largest_unit"),
}


def median(values):
    return round(statistics.median(values), 3) if values else None


def histogram_median(counts):
    total = sum(counts.values())
    if not total:
        return None
    positions = ((total - 1) // 2, total // 2)
    seen = 0
    selected = []
    for value, count in sorted(counts.items()):
        if seen <= positions[0] < seen + count:
            selected.append(value)
        if seen <= positions[1] < seen + count:
            selected.append(value)
        seen += count
    return sum(selected) / 2


def percentile(values, percent):
    if not values:
        return None
    ordered = sorted(values)
    return round(ordered[round((len(ordered) - 1) * percent)], 3)


def spearman(xs, ys):
    def ranks(values):
        ordered = sorted(range(len(values)), key=values.__getitem__)
        out = [0.0] * len(values)
        pos = 0
        while pos < len(values):
            end = pos + 1
            while end < len(values) and values[ordered[end]] == values[ordered[pos]]:
                end += 1
            average = (pos + end - 1) / 2
            for index in ordered[pos:end]:
                out[index] = average
            pos = end
        return out
    a, b = ranks(xs), ranks(ys)
    ma, mb = statistics.mean(a), statistics.mean(b)
    numerator = sum((x - ma) * (y - mb) for x, y in zip(a, b))
    denominator = math.sqrt(sum((x - ma) ** 2 for x in a) *
                            sum((y - mb) ** 2 for y in b))
    return round(numerator / denominator, 4) if denominator else None


def classify(rows, transport, size_only=False):
    """Stratified per-workload split and four-class standardized nearest centroid."""
    names = SIZE_ONLY_FEATURES[transport] if size_only else FEATURES[transport]
    by_workload = collections.defaultdict(list)
    for row in rows:
        by_workload[row["workload"]].append(row)
    train, test = [], []
    rng = random.Random(13)
    for workload in sorted(by_workload):
        group = sorted(by_workload[workload], key=lambda row: row["sample"])
        rng.shuffle(group)
        cut = min(len(group) - 1, max(1, int(.7 * len(group))))
        train.extend(group[:cut])
        test.extend(group[cut:])
    def vector(row):
        return [math.log1p(row[name]) for name in names]
    vectors = [vector(row) for row in train]
    centers = [statistics.mean(v[i] for v in vectors) for i in range(len(names))]
    scales = [statistics.pstdev(v[i] for v in vectors) or 1 for i in range(len(names))]
    def standard(row):
        return [(x - center) / scale for x, center, scale in
                zip(vector(row), centers, scales)]
    centroids = {}
    for label in sorted(set(WORKLOAD_CLASS.values())):
        members = [standard(row) for row in train if WORKLOAD_CLASS[row["workload"]] == label]
        centroids[label] = [statistics.mean(v[i] for v in members)
                            for i in range(len(names))]
    confusion = {label: {other: 0 for other in centroids} for label in centroids}
    for row in test:
        actual = WORKLOAD_CLASS[row["workload"]]
        value = standard(row)
        predicted = min(centroids, key=lambda label:
                        sum((x - y) ** 2 for x, y in zip(value, centroids[label])))
        confusion[actual][predicted] += 1
    correct = sum(confusion[label][label] for label in centroids)
    recalls = [confusion[label][label] / sum(confusion[label].values())
               for label in centroids]
    n = len(test)
    p = correct / n
    z = 1.96
    center = (p + z * z / (2 * n)) / (1 + z * z / n)
    half_width = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / (
        1 + z * z / n)
    return {
        "method": "log1p features; train-only standardization; Euclidean nearest four-class centroid",
        "size_only_ablation": size_only,
        "classes": WORKLOAD_CLASS, "features": list(names),
        "split": "deterministic seed 13, 70/30 per-workload independent connections",
        "train_count": len(train), "test_count": len(test),
        "accuracy": round(p, 4),
        "accuracy_wilson_95": [round(center - half_width, 4),
                               round(center + half_width, 4)],
        "balanced_accuracy": round(statistics.mean(recalls), 4),
        "majority_test_baseline": round(max(sum(v.values()) for v in confusion.values()) /
                                        len(test), 4),
        "confusion_actual_by_predicted": confusion,
    }


def public_fingerprint(rows, field):
    profiles = []
    for row in rows:
        value = row.get(field)
        if value is None:
            continue
        # Public metadata only; omit per-connection randoms and full key shares.
        stable = {
            "handshake_bytes": value["handshake_bytes"],
            "legacy_record_version": value["legacy_record_version"],
            "legacy_hello_version": value["legacy_hello_version"],
            "cipher_suites": value["cipher_suites"],
            "extension_order": value["extension_order"],
            "groups": value["supported_groups"],
            "signature_algorithms": value["signature_algorithms"],
            "key_share_groups_and_lengths": value["key_shares"],
            "supported_versions_raw": value["supported_versions_raw"],
            "sni_present": value["sni_present"],
            "alpn_present": value["alpn_present"],
        }
        profiles.append(stable)
    if not profiles:
        return {"sample_count": 0}
    def frequency(name):
        counts = collections.Counter(json.dumps(profile[name], sort_keys=True)
                                     for profile in profiles)
        return [{"value": json.loads(value), "count": count}
                for value, count in counts.most_common(5)]
    return {
        "sample_count": len(profiles),
        "unique_full_public_profile_count": len({
            json.dumps(profile, sort_keys=True) for profile in profiles}),
        "handshake_bytes": frequency("handshake_bytes"),
        "legacy_record_version": frequency("legacy_record_version"),
        "legacy_hello_version": frequency("legacy_hello_version"),
        "cipher_suite_lists": frequency("cipher_suites"),
        "extension_order_unique_count": len({
            tuple(profile["extension_order"]) for profile in profiles}),
        "extension_order_top_five": frequency("extension_order"),
        "extension_type_sets": [
            {"value": list(value), "count": count} for value, count in
            collections.Counter(tuple(sorted(profile["extension_order"]))
                                for profile in profiles).most_common(5)],
        "supported_groups": frequency("groups"),
        "signature_algorithms": frequency("signature_algorithms"),
        "key_share_groups_and_lengths": frequency("key_share_groups_and_lengths"),
        "supported_versions_raw": frequency("supported_versions_raw"),
        "sni_present": frequency("sni_present"),
        "alpn_present": frequency("alpn_present"),
    }


def summarize(rows):
    per_transport = {}
    for transport in sorted({row["transport"] for row in rows}):
        subset = [r for r in rows if r["transport"] == transport]
        by_workload = {}
        for workload in WORKLOAD_CLASS:
            group = [r for r in subset if r["workload"] == workload]
            if not group:
                continue
            app_up, app_down = APP_BYTES[workload]
            app_total = app_up + app_down
            by_workload[workload] = {
                "n": len(group),
                "median_wire_c2s_bytes": median([r["total_c2s"] for r in group]),
                "median_wire_s2c_bytes": median([r["total_s2c"] for r in group]),
                "median_unit_count_c2s": median([r["count_c2s"] for r in group]),
                "median_unit_count_s2c": median([r["count_s2c"] for r in group]),
                "median_duration_ms": median([r["duration_ms"] for r in group]),
                "median_response_ms": median([r["time_to_first_response_ms"]
                                              for r in group
                                              if r["time_to_first_response_ms"] is not None]),
                "p90_response_ms": percentile([r["time_to_first_response_ms"]
                                                for r in group
                                                if r["time_to_first_response_ms"] is not None], .9),
                "median_secure_handshake_to_target_ms": median(
                    [r["secure_handshake_to_target_ms"] for r in group
                     if r["secure_handshake_to_target_ms"] is not None]),
                "median_burst_count": median([r["burst_count"] for r in group]),
                "median_largest_unit": median([r["largest_unit"] for r in group]),
                "median_direction_changes": median([r["direction_changes"] for r in group]),
                "median_tls_record_count": median([r["tls_record_count"] for r in group]),
                "median_wire_to_app_bytes_ratio": median(
                    [(r["total_c2s"] + r["total_s2c"]) / app_total for r in group])
                if app_total else None,
                "median_idle_units_after_target_accept": median(
                    [r["idle_units_after_target_accept"] for r in group
                     if r["idle_units_after_target_accept"] is not None]),
                "first_four_lengths_each_direction_from_first_16": {
                    side: [entry["wire_len"] for entry in group[0]["first_16"]
                           if entry["direction"] == side][:4] for side in ("c2s", "s2c")
                },
            }
        sized = [r for r in subset if sum(APP_BYTES[r["workload"]]) > 0]
        size = [sum(APP_BYTES[r["workload"]]) for r in sized]
        correlations = {
            name: spearman(size, [r[name] for r in sized])
            for name in ("total_c2s", "total_s2c", "count_c2s", "count_s2c",
                         "largest_unit", "burst_count")
        }
        correlations["total_wire_bytes"] = spearman(
            size, [r["total_c2s"] + r["total_s2c"] for r in sized])
        correlations["total_observed_units"] = spearman(
            size, [r["count_c2s"] + r["count_s2c"] for r in sized])
        if transport == "tcp":
            correlations["tls_record_count"] = spearman(
                size, [r["tls_record_count"] for r in sized])
        initial_headers = collections.Counter(
            (header["version"], header["long_packet_type"], header["dcid_len"],
             header["scid_len"], header["token_len"], header["datagram_len"])
            for row in subset for header in row["quic_initial_headers"])
        first_tls_record_lengths = collections.Counter(
            length for row in subset for length in row["tls_record_lengths_first_16"])
        per_transport[transport] = {
            "sample_count": len(subset), "workloads": by_workload,
            "size_leakage_spearman": correlations,
            "classifier": classify(subset, transport),
            "classifier_size_only": classify(subset, transport, size_only=True),
            "client_hello_fingerprints": public_fingerprint(subset, "tls_client_hello"),
            "server_hello_fingerprints": public_fingerprint(subset, "tls_server_hello"),
            "public_quic_initial_headers": [
                dict(zip(("version", "packet_type", "dcid_len", "scid_len",
                          "token_len", "datagram_len"), key), count=count)
                for key, count in initial_headers.most_common()],
            "sampled_first_16_tls_record_lengths": first_tls_record_lengths.most_common(20),
        }
    return per_transport


def verify_and_summarize_unit_files(rows, directory, transports, burst_gap_ms):
    gaps = collections.defaultdict(list)
    bursts = collections.defaultdict(list)
    unit_lengths = collections.defaultdict(collections.Counter)
    record_lengths = collections.defaultdict(collections.Counter)
    record_types = collections.defaultdict(collections.Counter)
    application_record_lengths = collections.defaultdict(collections.Counter)
    prefix_records = collections.defaultdict(collections.Counter)
    for row in rows:
        path = directory / f"{row['id']}.units.csv"
        assert hashlib.sha256(path.read_bytes()).hexdigest() == row["unit_metadata_sha256"]
        with path.open(newline="") as stream:
            observed = [(int(unit["timestamp_us_from_first"]), unit["direction"],
                         int(unit["wire_length"])) for unit in csv.DictReader(stream)]
        assert len(observed) == row["count_c2s"] + row["count_s2c"]
        assert observed[0][0] == 0
        assert all(observed[i][0] >= observed[i - 1][0]
                   for i in range(1, len(observed)))
        for side in ("c2s", "s2c"):
            assert sum(length for _, direction, length in observed if direction == side) == (
                row["total_c2s"] if side == "c2s" else row["total_s2c"])
        assert [{"direction": side, "wire_len": length}
                for _, side, length in observed[:16]] == row["first_16"]
        key = (row["transport"], row["workload"])
        burst_size = 0
        for index, (timestamp, _, length) in enumerate(observed):
            unit_lengths[key][length] += 1
            if index:
                gap = (timestamp - observed[index - 1][0]) / 1000
                gaps[key].append(gap)
                if gap > burst_gap_ms:
                    bursts[key].append(burst_size)
                    burst_size = 0
            burst_size += length
        bursts[key].append(burst_size)
        record_path = directory / f"{row['id']}.tls_records.csv"
        with record_path.open(newline="") as stream:
            records = [(int(item["record_length"]), int(item["tls_content_type"]))
                       for item in csv.DictReader(stream)]
        assert len(records) == row["tls_record_count"]
        for length, kind in records:
            record_lengths[key][length] += 1
            record_types[key][kind] += 1
            if kind == 23:
                application_record_lengths[key][length] += 1
            if length == 26 and kind == 23:
                prefix_records[key]["26_byte_application_records"] += 1
    for transport in transports:
        per_workload = {}
        for workload in WORKLOAD_CLASS:
            key = (transport, workload)
            per_workload[workload] = {
                "interarrival_p50_ms": median(gaps[key]),
                "interarrival_p90_ms": percentile(gaps[key], .9),
                "interarrival_p99_ms": percentile(gaps[key], .99),
                "burst_size_p50_bytes": median(bursts[key]),
                "burst_size_p90_bytes": percentile(bursts[key], .9),
                "unit_length_top_ten": unit_lengths[key].most_common(10),
                "tls_record_length_top_ten": record_lengths[key].most_common(10),
                "tls_record_content_type_counts": dict(record_types[key]),
                "tls_26_byte_application_record_count": prefix_records[key][
                    "26_byte_application_records"],
                "tls_application_record_count": sum(application_record_lengths[key].values()),
                "tls_application_record_median_size": histogram_median(
                    application_record_lengths[key]),
                "tls_application_record_at_most_32_count": sum(
                    count for length, count in application_record_lengths[key].items()
                    if length <= 32),
                "tls_application_record_at_most_32_histogram": [
                    [length, count] for length, count in
                    sorted(application_record_lengths[key].items()) if length <= 32],
            }
        transports[transport]["full_unit_metadata"] = per_workload
        transports[transport]["all_unit_count"] = sum(
            row["count_c2s"] + row["count_s2c"] for row in rows
            if row["transport"] == transport)
        transports[transport]["all_tls_record_count"] = sum(
            row["tls_record_count"] for row in rows if row["transport"] == transport)
        transports[transport]["all_tls_application_record_count"] = sum(
            sum(application_record_lengths[(transport, workload)].values())
            for workload in WORKLOAD_CLASS)
        transports[transport]["all_tls_26_byte_application_record_count"] = sum(
            prefix_records[(transport, workload)]["26_byte_application_records"]
            for workload in WORKLOAD_CLASS)
        transports[transport]["all_tls_application_record_at_most_32_count"] = sum(
            count for workload in WORKLOAD_CLASS
            for length, count in application_record_lengths[(transport, workload)].items()
            if length <= 32)
        transports[transport]["all_tls_application_record_at_most_32_histogram"] = [
            [length, count] for length, count in sorted(sum(
                (application_record_lengths[(transport, workload)]
                 for workload in WORKLOAD_CLASS), collections.Counter()).items())
            if length <= 32]
        transports[transport]["all_tls_application_record_length_top_ten"] = sum(
            (application_record_lengths[(transport, workload)]
             for workload in WORKLOAD_CLASS), collections.Counter()).most_common(10)


def main(args):
    source = pathlib.Path(args.input)
    data = json.loads(source.read_text())
    rows = data["rows"]
    expected_transports = ({"tcp", "quic"} if data.get("transport_selection", "both") == "both"
                           else {"tcp"})
    assert len(rows) == len(expected_transports) * (
        9 * data["sample_counts"]["regular_per_workload_per_transport"] +
        data["sample_counts"]["bulk_per_transport"])
    assert {r["transport"] for r in rows} == expected_transports
    for row in rows:
        assert all(row["first_16"]) and row["count_c2s"] and row["count_s2c"]
    output = pathlib.Path(args.output_dir)
    output.mkdir(parents=True, exist_ok=True)
    summary = {
        "schema": 1,
        "capture_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "source_head": data["source_head"], "lock_sha256": data["lock_sha256"],
        "capture_unit": data["capture_unit"], "burst_gap_ms": data["burst_gap_ms"],
        "sample_counts": data["sample_counts"],
        "workload_application_bytes_up_down": APP_BYTES,
        "transports": summarize(rows),
    }
    verify_and_summarize_unit_files(
        rows, source.with_name(source.name + ".units"), summary["transports"],
        data["burst_gap_ms"])
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    with (output / "features.csv").open("w", newline="") as stream:
        fields = ["id", "transport", "workload", "sample", "class", "app_up", "app_down",
                  "total_c2s", "total_s2c", "count_c2s", "count_s2c",
                  "duration_ms", "time_to_first_response_ms",
                  "secure_handshake_to_target_ms", "interarrival_median_ms",
                  "interarrival_p90_ms", "burst_count", "largest_unit",
                  "smallest_nonempty_unit", "mean_unit", "median_unit",
                  "direction_changes", "up_down_byte_ratio", "tls_record_count",
                  "idle_units_after_target_accept", "first_16_lengths_with_direction",
                  "first_four_c2s", "first_four_s2c",
                  "size_histogram_cumulative", "burst_bytes_first_32",
                  "unit_metadata_sha256"]
        writer = csv.DictWriter(stream, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        for row in rows:
            out = {field: row.get(field) for field in fields}
            out["class"] = WORKLOAD_CLASS[row["workload"]]
            out["app_up"], out["app_down"] = APP_BYTES[row["workload"]]
            out["first_16_lengths_with_direction"] = json.dumps(row["first_16"],
                                                                separators=(",", ":"))
            for side in ("c2s", "s2c"):
                out["first_four_" + side] = json.dumps(
                    [entry["wire_len"] for entry in row["first_16"]
                     if entry["direction"] == side][:4], separators=(",", ":"))
            out["size_histogram_cumulative"] = json.dumps(
                row["size_histogram_cumulative"], separators=(",", ":"))
            out["burst_bytes_first_32"] = json.dumps(row["burst_bytes"], separators=(",", ":"))
            writer.writerow(out)
    for transport, part in summary["transports"].items():
        print(transport, "samples", part["sample_count"], "accuracy",
              part["classifier"]["accuracy"], "balanced",
              part["classifier"]["balanced_accuracy"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input")
    parser.add_argument("--output-dir", default="tests/stage13_traffic_morphology")
    main(parser.parse_args())
