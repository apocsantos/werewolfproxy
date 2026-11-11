# Stage47 M3 — regression

**M3: PASS.** The single-address fast path (M2, O1) changes no
behaviour: the Stage46 matrix matches Stage46 M3 on every scenario, the
Stage42 floods behave as before with an encrypted-TCP Fang out of
descriptors at 1.62–1.74 s of CPU per episode (the pre-Stage46 level),
the Stage32 cells pass 3/3, and `ci-local` passes 13/13.

## 1. Baseline

- `main` `e3d89f9` (PR #147 merged). Lab debug `werewolfd` `d226a528…`,
  the same binary as the M2 A/B.
- Run 2026-11-11 17:14–17:46 UTC, in this order: matrix, floods (three
  rounds), chaos, `ci-local`. `/tmp` stayed at 8%; the log guard never
  triggered; the daemons stayed up in every run.
- No source or tooling change in this PR.

Results in `target/stage47-m3/` (`meta.txt` has the order and times);
`ci-local` in `target/ci-local/20261111T174013Z`.

## 2. Stage46 matrix (`tools/stage46/dial.py`), one round

40 scenarios, 5 clients each. Every scenario matches Stage46 M3 (§2 of
that record) within 0.05 s:

| Path | `fast` | `silent` | `family` | `dead` |
|---|---|---|---|---|
| QUIC | echo, 0.30–0.31 | echo, 0.29–0.31 | echo, 0.28–0.31 | reset, 5.0; `dial` |
| encrypted TCP | echo, 0.01–0.02 | echo, 0.28–0.30 | echo, 0.01–0.02 | reset, 10.0 (F1 retry); `dial` |
| plain TCP | echo, 0.00 | echo, 0.25 | echo, 0.00 | reset, 5.0; `target` |
| failover (QUIC provider by name) | echo, 0.30–0.31 | echo, 0.28–0.30 | echo, 0.30–0.32 | echo via C, 5.04–5.06 |
| routed, QUIC hop | echo, 0.34–0.36 | echo, 0.33–0.35 | echo, 0.32–0.35 | reset, 5.0; `dial` |
| routed, TCP hop | echo, 0.08–0.13 | echo, 0.35–0.40 | echo, 0.09–0.13 | reset, 5.0; `dial` |
| discovery, QUIC / TCP | `ok`, 0.32 / 0.02 | `ok`, 0.32 / 0.28 | `ok`, 0.33 / 0.02 | `unreachable`, 5.01 |
| sync, QUIC / TCP | `ok`, 0.35 / 0.02 | `ok`, 0.36 / 0.33 | `ok`, 0.35 / 0.02 | `unreachable`, 5.01 |

`sessions_failed` is 0 wherever every client echoed and 6 where all 5
failed (the readiness probe counts, Stage45). The hostname scenarios
resolve to several addresses and so still take the race; the `fast`,
`silent` and `family` results show it is unchanged.

## 3. Stage42 floods (`tools/stage42/fang_flood.py`), three rounds

### `nofile` 256

| Scenario | Descriptors flooded (r1/r2/r3) | `cpu_seconds_flood` | Log | Released | Legit after |
|---|---|---|---|---|---|
| `fd:quic` | 74 / 76 / 82 | 0.99 / 0.97 / 1.11 | 190 bytes | 18 | PASS ×3 |
| `fd:tcp` | 145 ×3 | 0.65 / 0.81 / 0.64 | 297 bytes | 17 | PASS ×3 |
| `fd:tcp-plain` | 145 ×3 | 0.06 / 0.05 / 0.06 | 89 bytes | 17 | PASS ×3 |

Listening throughout; 0% CPU in the 5 s window after the flood, as in
Stage46 M3.

| Scenario | Echoed (r1/r2/r3) | Descriptors flooded | `cpu_seconds_flood` | Legit during | Legit after |
|---|---|---|---|---|---|
| `quota:quic` | 57 / 60 / 63 | 75 / 78 / 81 | 0.95–0.99 | PASS / PASS / PASS | PASS ×3 |
| `quota:tcp` | 48 / 64 / 61 | 113 / 145 / 139 | 0.67–0.70 | PASS / FAIL / PASS | PASS ×3 |
| `quota:tcp-plain` | 64 ×3 | 145 ×3 | 0.05–0.08 | FAIL ×3 | PASS ×3 |

"Legit during" follows the echoed count, as in Stage42–46 M3: it fails
exactly when the flood holds all 64 slots (the stated Stage42 M2 limit,
an active flood from loopback). Every run is at 17–18 descriptors after
the release.

### `nofile` 64

| Scenario | `cpu_seconds_flood` (r1/r2/r3) | `cpu_flooded_pct` (5 s) | Log | Flood | Recovered, legit after |
|---|---|---|---|---|---|
| `fd:quic` | 1.07 / 0.77 / 0.78 | 1.4–2.0% | 295 bytes | 5.5–6.0 s | yes ×3 |
| `fd:tcp` | **1.74 / 1.62 / 1.74** | 1.6–1.8% | 1,358–1,801 bytes | 29.3–30.7 s | yes ×3 |

`fd:tcp` (mean 1.70 s) sits within the M2 A/B's range for both O1 and the
pre-Stage46 build (1.61–1.74 s), and below M1's Stage46 figure (1.82 s,
1.84–1.88 without its short run). `fd:quic` matches M1 and M2
(0.73–0.96 s). The 5 s figure stays within 1.4–2.0% on all six runs.

## 4. Stage32 cells (`chaos.py network --cells S2:P0,S3:P0,S4:P0`)

| Cell | M3 | Historical | Detail (runs 1–3) | Stage46 M3 |
|---|---|---|---|---|
| S2-P0 | PASS 3/3 | PASS | 0.503–0.539 s, exact match | 0.536–0.541 s |
| S3-P0 | PASS 3/3 | PASS | QUIC hop 0.80–0.82 s; TCP hop 1.14–1.21 s | 0.79–0.85 s; 1.15–1.21 s |
| S4-P0 | PASS 3/3 | PASS | 50/50 per transport; max QUIC 0.014 s, TCP 0.008 s | 0.014–0.016 s; 0.007–0.008 s |

The chaos paths dial literal addresses, so every dial here took the fast
path.

## 5. Gates (`scripts/ci-local.sh` full)

**13 of 13 PASS**, including the audit, the acceptance lab and the
discovery, sync, routed-Fang, provider and service gates.

## 6. Conclusion

All M3 criteria hold. M4 is the `KNOWN_LIMITATIONS` update and the
closure. To record there: the Stage46 CPU item is resolved (M2 applied R1
and removed the cost; M3 confirms 1.62–1.74 s per episode across three
more runs), and the Stage42 floods now report `cpu_seconds_flood`, the
figure later stages should compare.
