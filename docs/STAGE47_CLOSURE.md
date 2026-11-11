# Stage47 — closure record

**Status: Stage47 CLOSED.** The CPU observation Stage46 recorded under
descriptor exhaustion was a 5 s sampling artefact over a real but small
cost: about 12% more CPU per episode on an encrypted-TCP Fang, none on
QUIC, none in steady state. A single address is now dialled without the
race's task machinery, which removes it: 1.62–1.74 s per episode, the
pre-Stage46 level. The Stage42 flood tool now has a cumulative CPU
metric, and the record states the measured figures.

This record is documentation only: no production source, harness,
dependency, bound or historical verdict changes.

## 1. Baseline

- `main` after PR #148 (M3) merged.
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18, rustix 1.1.6. No
  `Cargo.lock` change in Stage47.
- Tag `v1.0.0-rc.2` is frozen and unchanged. Neither the Stage46 cost nor
  the Stage47 fix is in the rc.2 archive.
- S3-P2 and S2-P4 stay EXCLUDED. Stage38 stays CLOSED.
- No wire, trust, Target Policy or receiver change.

## 2. Milestones

| Step | PR | Result | Record |
|---|---|---|---|
| Survey | #145 | P1 the record overstates the cost (0.8–8.8% from a 5 s sample; about 11% per episode measured), P2 the Stage42 metric is timing-sensitive, P3 a small per-dial overhead. Recommended R1 + T1 + O1. S47-D0–D3 approved as recommended | [survey](STAGE47_FLOOD_CPU_SURVEY.md) |
| M1 test | #146 | P2, P3 CONFIRMED with T1 (`cpu_seconds_flood`): `fd:tcp` 1.82 s against 1.63 s before Stage46 M2, ranges disjoint; `fd:quic` 0.85 s against 0.84 s | [M1](STAGE47_M1_CPU_REPRODUCTION.md) |
| M2 fix | #147 | M2-PASS (`e3d89f9`): O1, `race` awaits a single address directly; R1 in `KNOWN_LIMITATIONS.md`. A/B `fd:tcp` 1.66 s against 1.67 s; O1 kept. M1's reasoning (0.27 ms per race too much for a task spawn) shown wrong. 454 tests; local CI 13/13 | [M2](STAGE47_M2_CPU_FIX.md) |
| M3 regression | #148 | PASS: Stage46 matrix ×1 unchanged; Stage42 floods ×3 unchanged, `fd:tcp` 1.74 / 1.62 / 1.74 s; Stage32 S2, S3, S4 P0 3/3; local CI 13/13 | [M3](STAGE47_M3_REGRESSION.md) |
| M4 | this PR | `KNOWN_LIMITATIONS.md` updated; closure | — |

## 3. What remains open

- **Why the race cost that much:** one spawned task and `select!` per
  dial cost about 0.27 ms under descriptor exhaustion; not investigated
  (`perf` is not installed). A hostname with several addresses still
  takes the race and so may still pay it; no measured case.
- **Earlier comparisons:** Stage43–46 flood comparisons used the 5 s
  `cpu_flooded_pct`; their CPU figures are indicative only. Later stages
  should compare `cpu_seconds_flood`.
- From Stage46: an encrypted-TCP Fang where no address answers takes 10 s
  per client (the Stage32 F1 retry repeats the race); QUIC resolves a name
  only at open; the dialer was run only in the namespace sandbox. From
  Stage45: a dead QUIC peer costs each client the 5 s window; refusal
  reasons stay hidden from the initiator, by design. From Stage44: real
  IPv6 networks, the /64 share heuristic, failover Fangs over IPv6. From
  Stage43: the active loopback flood (F2), the shared peer quota (F5), the
  operator ceiling per Fang (F4) and half-close. From earlier stages: the
  initiator's stream windows (Stage41; the send window is the Stage34
  surface), the routed-QUIC regression root cause (Stage34), and the
  real-WAN relay-to-destination check (S39-D5).

**Not authorised:** tokio promotion, rc.3, release certification, WAN
testing.
