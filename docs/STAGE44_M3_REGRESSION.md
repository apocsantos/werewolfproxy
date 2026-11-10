# Stage44 M3 — Regression

**M3: PASS.** The M1 IPv6 scenarios pass in all three rounds on the M2
build. The IPv4 results that Stage44 could have disturbed hold:

- the Stage43 idle and source scenarios;
- the Stage42 flood scenarios;
- the Stage32 QUIC cells;
- the routed-Fang gate.

Every historical verdict holds.

## 1. Build

- Tested commit `0f9ffa5` (M2 branch head). Its tree is identical to
  `main` `48aabda` (the #131 merge); `git diff 0f9ffa5 48aabda` is empty.
- Lab debug `werewolfd`
  `a5d259d32695dd919f05b9edfd0a25d4de273296b0ea435648f4a3082dea4a96`,
  the M2 binary. Every result file records that hash.
- Run 2026-11-10 16:43–17:10 UTC. `/tmp` stayed at 5%; the log guard
  never triggered; the daemons stayed up in every run.
- No source or tooling change in this PR.

Results:

- `target/stage44-m3/`: `r{1,2,3}-ipv6.json`, `s43-*.json`, `s42-*.json`,
  logs and `meta.txt`;
- chaos: `target/stage32-chaos/results/20261110T165742Z-network`;
- routed gate: `target/stage44-m3/routed.log`.

## 2. IPv6 scenarios (`tools/stage44/ipv6.py`), three rounds

| Scenario | M1 (`main` before M2) | M3 r1 / r2 / r3 |
|---|---|---|
| I1 `fang:quic` | echo reset; 1 MiB 0 bytes | echo PASS, 1 MiB PASS (SHA-256) ×3 |
| I2 `fang:tcp`, `fang:tcp-plain` | PASS | PASS ×3 |
| I3 `discovery:quic` | `rejected` | `ok`, 1 record ×3 |
| I3 `discovery:tcp` | `ok` | `ok` ×3 |
| I3 `sync:quic` | `rejected`, head 0 | applied [1], head 1 ×3 |
| I3 `sync:tcp` | applied [1] | applied [1] ×3 |
| I4 `routed:quic` | echo reset | `tcp via wolf-b (quic)`, echo and 1 MiB PASS ×3 |
| I4 `routed:tcp` | PASS | PASS ×3 |
| I5 `dualstack` | IPv6 QUIC reset | IPv6 QUIC/TCP and IPv4 QUIC/TCP PASS ×3 |
| I6 `share:tcp`, `share:tcp-plain` | one /64 held 64; other /64 reset | first address 16, others 0; other /64 PASS during and after ×3 |
| I7 `cli` | `--bind ::1` → `::1:P` | `[::1]:P` for both spellings ×3 |

## 3. IPv4 flood scenarios, one round

These rerun the V2 share and the shared acceptor over IPv4.

| Scenario | Result | Stage43 M3 |
|---|---|---|
| Stage43 `idle:*` (quic, tcp, tcp-plain) | held 64; at ceiling reset (intended); after 65 s idle PASS; after release PASS | same |
| Stage43 `source:*` | flood from 10.43.0.2 held **16**; 10.43.0.3 PASS at once, after 30 s active, after release | same |
| Stage42 `fd:*`, `nofile` 256 | listening throughout; 82 / 145 / 145 descriptors; 0% CPU; 17–18 after release; legit PASS | same |
| Stage42 `fd:quic`, `fd:tcp`, `nofile` 64 | 64 / 63 descriptors; 1.8% / 2.0% CPU; 188 / 428 bytes of log; recovered, legit PASS | same (1.4–2.0%) |
| Stage42 `quota:*` | echoed 64 / 63 / 64; legit during FAIL / PASS / FAIL; after PASS; 17–18 descriptors at 5 s and 30 s | same pattern (Stage43 M3 §3) |

The IPv4 source share is still per address (16 for 10.43.0.2). "Legit
during" in the quota scenarios still follows whether the active loopback
flood filled all 64 slots. That is the limit stated in Stage43, not a
regression.

## 4. Stage32 cells (`chaos.py network`), because the QUIC dial path changed

| Cell | M3 | Historical | Detail (runs 1–3) | Stage43 M3 |
|---|---|---|---|---|
| S2-P0 | PASS 3/3 | PASS | 0.53–0.54 s, exact match | 0.50–0.52 s |
| S2-P1 | PASS 3/3 | PASS | 0.51–0.56 s | 0.52–0.56 s |
| S3-P0 | PASS 3/3 | PASS | QUIC hop 0.78–0.85 s; TCP hop 1.16–1.23 s | 0.79 s; 1.18–1.20 s |
| S4-P0 | PASS 3/3 | PASS | 50/50 per transport; max QUIC 0.015 s | 0.014 s |
| S7-P0 | PASS 3/3 | PASS | 10/10 selections QUIC | same |

These cells run over IPv4, so they take the unchanged `0.0.0.0:0`
endpoint. Their timings match Stage43 M3.

## 5. Routed-Fang gate (`tests/integration/routed_fang_gate.py`)

PASS, 12 of 12. This includes 64 MiB full duplex over a QUIC hop and an
encrypted-TCP hop, Silver on the relay cutting a live routed session, a
relay address change closing the routed Fang, and plain TCP refused for a
routed peer.

## 6. Conclusion

All M3 criteria hold. M4 is the `KNOWN_LIMITATIONS` update (IPv6 moves
from "Not run" to what was run in the sandbox) and the closure. What
remains:

- IPv6 run only in a namespace sandbox on loopback-class addresses;
- dialling uses the first resolved address only;
- the /64 heuristic;
- a QUIC dial failure is still not logged, and discovery and sync report
  it as `rejected`.
