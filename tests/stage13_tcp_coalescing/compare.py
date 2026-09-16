#!/usr/bin/env python3
"""Compare two metadata-only TCP captures using the frozen Stage13A features."""

import argparse
import hashlib
import json
import pathlib


def metrics(summary):
    tcp = summary["transports"]["tcp"]
    records = tcp["all_tls_record_count"]
    short = tcp["all_tls_application_record_at_most_32_count"]
    application = tcp["all_tls_application_record_count"]
    exact_26 = tcp["all_tls_26_byte_application_record_count"]
    return {
        "capture_sha256": summary["capture_sha256"],
        "source_head_recorded_by_capture": summary["source_head"],
        "connections": tcp["sample_count"],
        "tls_records": records,
        "tls_application_records": application,
        "tls_exact_26_byte_application_records": exact_26,
        "exact_26_percent_of_all_records": round(100 * exact_26 / records, 3),
        "tls_application_records_at_most_32_bytes": short,
        "at_most_32_percent_of_application_records": round(100 * short / application, 3),
        "at_most_32_histogram": tcp["all_tls_application_record_at_most_32_histogram"],
        "tls_application_record_length_top_ten": tcp[
            "all_tls_application_record_length_top_ten"],
        "spearman_application_size_wire_bytes": tcp["size_leakage_spearman"]["total_wire_bytes"],
        "spearman_application_size_tls_record_count": tcp[
            "size_leakage_spearman"]["tls_record_count"],
        "classifier_full_accuracy": tcp["classifier"]["accuracy"],
        "classifier_size_only_accuracy": tcp["classifier_size_only"]["accuracy"],
        "workloads": {
            name: {
                "connections": workload["n"],
                "median_wire_c2s_bytes": workload["median_wire_c2s_bytes"],
                "median_wire_s2c_bytes": workload["median_wire_s2c_bytes"],
                "median_tls_record_count": workload["median_tls_record_count"],
                "median_tls_application_record_size": tcp["full_unit_metadata"][name][
                    "tls_application_record_median_size"],
                "median_completion_ms": workload["median_duration_ms"],
                "median_first_response_ms": workload["median_response_ms"],
                "median_wire_to_application_ratio": workload[
                    "median_wire_to_app_bytes_ratio"],
                "exact_26_byte_application_records": tcp["full_unit_metadata"][name][
                    "tls_26_byte_application_record_count"],
            }
            for name, workload in tcp["workloads"].items()
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=pathlib.Path)
    parser.add_argument("after", type=pathlib.Path)
    parser.add_argument("--before-source", required=True, type=pathlib.Path)
    parser.add_argument("--after-source", required=True, type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path,
                        default=pathlib.Path("tests/stage13_tcp_coalescing/comparison.json"))
    args = parser.parse_args()
    before = json.loads(args.before.read_text())
    after = json.loads(args.after.read_text())
    assert before["lock_sha256"] == after["lock_sha256"]
    assert before["burst_gap_ms"] == after["burst_gap_ms"]
    assert before["sample_counts"] == after["sample_counts"]
    assert before["capture_unit"] == after["capture_unit"]
    assert before["workload_application_bytes_up_down"] == after[
        "workload_application_bytes_up_down"]
    assert set(before["transports"]) == set(after["transports"]) == {"tcp"}
    assert before["transports"]["tcp"]["sample_count"] == 275
    assert after["transports"]["tcp"]["sample_count"] == 275
    result = {
        "schema": 1,
        "method": "same Stage13A production relay, workloads, features, classifier and 275 samples",
        "lock_sha256": before["lock_sha256"],
        "production_tcp_source_sha256": {
            "before": hashlib.sha256(args.before_source.read_bytes()).hexdigest(),
            "after": hashlib.sha256(args.after_source.read_bytes()).hexdigest(),
        },
        "before": metrics(before),
        "after": metrics(after),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print("26-byte TLS application records:", result["before"][
        "tls_exact_26_byte_application_records"], "->", result["after"][
        "tls_exact_26_byte_application_records"])


if __name__ == "__main__":
    main()
