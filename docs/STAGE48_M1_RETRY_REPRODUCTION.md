# Stage48 M1 — the setup retry on a dead peer, reproduced

**M1: P1, P2 and P3 CONFIRMED.**

- **P1:** failover with a silent encrypted-TCP first provider reaches the
  next provider after **10.04–10.06 s**; with a silent QUIC provider,
  5.05–5.07 s.
- **P2:** with **two** silent encrypted-TCP providers before a live one,
  every client is **reset at 15.0 s**: the first provider's connect and
  its retry take 10 s, the second gets the remaining 5 s, and the
  selection window ends ("wolf-c: selection window ended"). The live
  third provider is never tried. With two silent QUIC providers the third
  serves every client at 10.04–10.06 s.
- **P3:** a direct encrypted-TCP Fang to a silent peer resets each client
  at 10.0 s ("tcp connect timeout" twice per client); QUIC 5.0 s.

A refused peer costs the encrypted-TCP paths nothing (0.00–0.02 s), as
the survey said. The only change is tooling: `tools/stage48/retry.py`.

## 1. Baseline

- `main` `4d80183` (PR #150 merged). Lab debug `werewolfd` `d226a528…`,
  the Stage47 binary.
- Owner decisions S48-D0–D2 as recommended: approve; O1 (a connect
  timeout is final in the failover leg only); P3 recorded, the direct
  Fang unchanged.

## 2. Tool

`tools/stage48/retry.py` (README beside it) runs in the Stage46 sandbox
and adds `refused.stage46` (`127.0.0.2`, nothing listens) and
`blackhole.stage46` (`10.46.0.9`, dropped) to Stage46's `dead.stage46`
(both). Paths: the direct Fang over TCP or QUIC; failover with B by name
then C live (`failover-*`); failover with B and C by name then D live
(`failover3-*`, a fourth daemon). Clients wait up to 30 s, so the
daemon's 15 s selection window ends a failover attempt, not the client.

## 3. Results (2026-11-11 19:09–19:17 UTC, 3 clients each)

Results in `target/stage48-m1/` (`m1.json`, `m1.log`, `meta.txt`).

| Path | `refused` | `blackhole` | `dead` |
|---|---|---|---|
| encrypted TCP, direct | reset, 0.00 s; `dial`, refused | **reset, 10.00–10.01 s**; `dial`, "tcp connect timeout" | **reset, 10.00–10.01 s**; same |
| QUIC, direct | reset, 5.0 s; `dial`, "no answer from peer" | reset, 5.0 s; same | reset, 5.0 s; same |
| failover, B over encrypted TCP, then C | echo via C, 0.01–0.02 s | **echo via C, 10.04 s** | **echo via C, 10.04–10.06 s** |
| failover, B over QUIC, then C | echo via C, 5.05–5.06 s | echo via C, 5.05–5.07 s | echo via C, 5.05–5.06 s |
| failover, B and C over encrypted TCP, then D | echo via D, 0.01–0.02 s | **reset, 15.0 s** ×3; "wolf-b: tcp connect timeout; wolf-c: selection window ended" | **reset, 15.0 s** ×3; same |
| failover, B and C over QUIC, then D | echo via D, 10.05–10.06 s | echo via D, 10.04–10.05 s | echo via D, 10.05–10.06 s |

- `sessions_failed` is 4 wherever all 3 clients were reset (the
  readiness probe counts, Stage45) and 0 wherever they echoed.
- A's log stays bounded: 1–3 lines per scenario, one class (`dial`).
- Every daemon stayed up in every scenario.
- A QUIC peer that refuses (nothing on the UDP port) also costs 5 s: QUIC
  does not act on the ICMP port-unreachable. That is the Stage45 item (a
  QUIC peer that does not answer costs each client the 5 s window), not
  the setup retry, and is outside Stage48.

## 4. Consequences for M2

O1 as approved: in the encrypted-TCP leg of failover, a TCP connect
timeout is final, so the next provider is tried at once. The other F1
cases (a TLS transport failure, a close or silence before the challenge)
stay retried there, and the direct Fang keeps every F1 retry. Criteria:

- `failover-tcp:blackhole` and `:dead` echo via C in 5.0–5.1 s;
- `failover3-tcp:blackhole` and `:dead` echo via D in 10.0–10.1 s, as the
  QUIC providers do;
- `tcp:blackhole` and `:dead` unchanged at 10.0 s (P3, recorded);
- refused unchanged everywhere.
