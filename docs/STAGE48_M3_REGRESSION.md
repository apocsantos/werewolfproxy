# Stage48 M3 — regression

**M3: PASS.** Across three rounds of the Stage48 tool, failover hands a
silent encrypted-TCP provider over at about 5 s and reaches a live third
provider behind two silent ones at about 10 s, exactly as QUIC providers
do. The direct Fang is unchanged (P3). The Stage46 matrix matches Stage47
M3, the Stage32 S2, S4 and S7 P0 cells pass 3/3, and `ci-local` passes
13/13.

## 1. Baseline

- `main` with PR #152 merged (M2, `3c7bbff`). Lab debug `werewolfd`
  `b928e6ff…`, the same binary as the M2 sandbox runs.
- Run 2026-11-11 21:22–22:02 UTC, in this order: the Stage48 tool ×3, the
  Stage46 matrix, chaos, `ci-local`. `/tmp` stayed at 8%; the log guard
  never triggered; the daemons stayed up in every run.
- No source or tooling change in this PR.

Results in `target/stage48-m3/` (`meta.txt` has the order and times);
`ci-local` in `target/ci-local/20261111T215546Z`.

## 2. Stage48 tool (`tools/stage48/retry.py`), three rounds, 3 clients each

Client times, rounds 1 / 2 / 3:

| Path | `refused` | `blackhole` | `dead` |
|---|---|---|---|
| encrypted TCP, direct | reset, 0.00 ×3 | reset, 10.00 / 10.00 / 10.00–10.01 | reset, 10.00 / 10.00–10.01 / 10.00–10.01 |
| QUIC, direct | reset, 5.0 ×3 | reset, 5.0 ×3 | reset, 5.0 ×3 |
| failover, B over encrypted TCP, then C | echo, 0.01–0.02 ×3 | **echo, 5.04–5.05 / 5.03–5.05 / 5.03–5.06** | **echo, 5.03–5.04 / 5.03–5.06 / 5.04–5.05** |
| failover, B over QUIC, then C | echo, 5.05–5.07 | echo, 5.04–5.06 | echo, 5.04–5.06 |
| failover, B and C over encrypted TCP, then D | echo, 0.01–0.02 | **echo, 10.04–10.05 / 10.04–10.05 / 10.03–10.05** | **echo, 10.03–10.04 / 10.03–10.04 / 10.05** |
| failover, B and C over QUIC, then D | echo, 10.04–10.06 | echo, 10.04–10.06 | echo, 10.03–10.06 |

M1 for comparison: one silent TCP provider 10.04–10.06 s; two, every
client reset at 15.0 s. Every round meets the M2 criteria. The direct
encrypted-TCP Fang stays at 10 s (P3, recorded by S48-D2).

## 3. Stage46 matrix (`tools/stage46/dial.py`), one round

40 scenarios, 5 clients each. Every scenario matches Stage47 M3 within
0.05 s:

| Path | `fast` | `silent` | `family` | `dead` |
|---|---|---|---|---|
| QUIC | echo, 0.31–0.32 | echo, 0.28–0.31 | echo, 0.30–0.31 | reset, 5.0; `dial` |
| encrypted TCP | echo, 0.01–0.02 | echo, 0.28–0.29 | echo, 0.01 | reset, 10.0 (F1 retry, unchanged); `dial` |
| plain TCP | echo, 0.00 | echo, 0.25 | echo, 0.00 | reset, 5.0; `target` |
| failover (QUIC provider by name) | echo, 0.30–0.31 | echo, 0.30–0.31 | echo, 0.30–0.31 | echo via C, 5.05–5.07 |
| routed, QUIC hop | echo, 0.34–0.35 | echo, 0.32–0.35 | echo, 0.33–0.36 | reset, 5.0; `dial` |
| routed, TCP hop | echo, 0.03–0.12 | echo, 0.36–0.40 | echo, 0.08–0.10 | reset, 5.0; `dial` |
| discovery, QUIC / TCP | `ok`, 0.31 / 0.02 | `ok`, 0.31 / 0.30 | `ok`, 0.31 / 0.02 | `unreachable`, 5.01 |
| sync, QUIC / TCP | `ok`, 0.35 / 0.02 | `ok`, 0.35 / 0.29 | `ok`, 0.31 / 0.02 | `unreachable`, 5.01 |

`sessions_failed` is 0 wherever every client echoed and 6 where all 5
failed (the readiness probe counts, Stage45).

## 4. Stage32 cells (`chaos.py network --cells S2:P0,S4:P0,S7:P0`)

| Cell | M3 | Historical | Detail (runs 1–3) | Stage47 M3 |
|---|---|---|---|---|
| S2-P0 | PASS 3/3 | PASS | 0.499–0.509 s, exact match | 0.503–0.539 s |
| S4-P0 | PASS 3/3 | PASS | 50/50 per transport; max QUIC 0.014–0.015 s, TCP 0.007–0.008 s | 0.014 s; 0.008 s |
| S7-P0 | PASS 3/3 | PASS | 10/10 selections QUIC | (Stage46 M3: same) |

## 5. Gates (`scripts/ci-local.sh` full)

**13 of 13 PASS**, including the audit, the acceptance lab and the
provider, service, routed-Fang, discovery and sync gates.

## 6. Conclusion

All M3 criteria hold. M4 is the `KNOWN_LIMITATIONS` update and the
closure. To record there: failover no longer redials a provider that did
not answer (P1, P2 fixed); a direct encrypted-TCP Fang to a peer that
does not answer still resets each client at 10 s (P3, by owner decision);
failover gives up one connect chance per provider under heavy loss (M2
§5).
