# Stage49 M3 — regression

**M3: PASS.** Three rounds of the Stage49 tool confirm that a refused
QUIC peer fails at once on every dial path. Failover reaches a live
provider behind one or two refused providers in 0.02–0.03 s; a refused
first address no longer costs the 250 ms stagger. Silent peers and TCP
paths retain their existing behaviour. The Stage46 and Stage48 matrices,
Stage42 QUIC floods, Stage32 S2/S3/S4/S7 P0 cells and full local gates
meet the M3 criteria.

## 1. Baseline and evidence recovery

- Tested commit `267385fd8ac54963c20f13501ac09be8f4684422` (Stage49 M2),
  on `stage49/m3-regression`; no production source or tooling change.
- Lab debug `werewolfd`
  `c484a543b6b049e5791105a2c6057d5b2ea076bd68a9c5e4d4b52e61da4d933a`,
  the M2 binary. Every Stage49, Stage46, Stage48 and Stage42 JSON record
  identifies this hash; the binary still matches it after recovery.
- Scenario runs completed on 2026-11-12, 08:14–08:51 UTC, before a host
  crash. Their complete JSON results, logs and successful exit records
  survived in `target/stage49-m3/`; `meta.txt` records the sequence.
  Recovery checked their scenario counts, outcomes, daemon survival,
  binary hashes and the Stage32 campaign summary.
- The interrupted `target/ci-local/20261112T085104Z` contains empty logs
  and no verdict, so it is not counted. A recovery attempt inside the
  execution sandbox (`20261112T090954Z`) failed on the read-only advisory
  cache and disposable daemons' filesystem checks. The approved host
  rerun, `target/ci-local/20261112T091031Z`, supplies the final gate
  results below. No test criteria or production checks were changed.

## 2. Stage49 tool, three rounds

`tools/stage49/refused.py`: 31 scenarios per round, 3 clients per
connection scenario. Ranges across all three rounds:

| QUIC path | `refused` | `blackhole` | `fast` |
|---|---|---|---|
| direct | reset, **0.00 s** | reset, 5.00 s | echo, 0.02–0.03 s |
| failover, one unavailable provider then C | echo, **0.02–0.03 s** | echo, 5.02–5.04 s | echo, 0.02–0.03 s |
| failover, two unavailable providers then D | echo, **0.02–0.03 s** | echo, 10.03 s | echo, 0.02–0.03 s |
| routed | reset, **0.00 s** | reset, 5.00 s | echo, 0.03–0.04 s |
| discovery | `unreachable`, **0.01 s** | `unreachable`, 5.01 s | `ok`, 0.03 s |
| governance sync | `unreachable`, **0.01 s** | `unreachable`, 5.01 s | `ok`, 0.04 s |

All refused paths meet the under-0.1 s criterion. M1 took 5 s per
refused QUIC provider and 0.30–0.36 s for `fast`. TCP controls retain
their M1 behaviour: refusal in 0.00–0.01 s; direct blackhole in
10.00–10.01 s; routed/discovery/sync blackhole in 5.00–5.01 s.

In each round, a live receiver discarded 10,000 one-byte datagrams with
**zero log lines**. Sending took 0.04 s, receiver CPU 0.08–0.09 s, and
all 6 clients before/after echoed. Every daemon survived every scenario.

Evidence: `r{1,2,3}-refused.json` and matching logs.

## 3. Stage46 and Stage48 matrices

Stage46 (`dial.json`): **40 scenarios, 5 clients each**, with the expected
results on every path. QUIC `fast`/`family` now reaches the live address
in 0.02–0.06 s across direct, failover, routed, discovery and sync paths.
QUIC `silent` still waits for the stagger (0.28–0.35 s); `dead` still
costs 5.00–5.01 s, or 5.04–5.07 s to fail over. Direct encrypted TCP
`dead` still costs 10.00–10.01 s (Stage48 P3). All daemons survived.

Stage48 (`retry.json`): **18 scenarios, 3 clients each**. With a silent
encrypted-TCP provider, failover reaches C in 5.03–5.06 s and D behind
two silent providers in 10.03–10.06 s. QUIC matches those windows.
Refused QUIC providers now fail over in 0.02–0.03 s; direct refusal
resets at 0.00 s. Direct silent TCP still resets at 10.00–10.01 s;
direct silent QUIC at 5.00 s. All daemons survived.

## 4. Stage42 QUIC floods

`tools/stage42/fang_flood.py`, with the extra probe socket in this build:

| Scenario | Descriptors during flood | CPU during saturation | After release |
|---|---|---|---|
| `fd:quic`, `nofile` 64, three rounds | 64 / 64 / 64 | 1.6 / 1.4 / 1.6% | 18 descriptors; listening, Fang listed, legitimate echo, daemon alive in every round |
| `fd:quic`, `nofile` 256 | 82 | 0% | 18 descriptors; listening, Fang listed, legitimate echo, daemon alive |
| `quota:quic`, 500 clients | 82; 64 echoed, 435 closed, 1 timeout | 0.91 s CPU over flood | 18 descriptors at 5 s and 30 s; legitimate echo, daemon alive |

At `nofile` 64 the listener pauses while descriptors are exhausted and
recovers after release; log growth was 188–295 bytes. At quota saturation
a legitimate connection during the flood fails, as in Stage42's stated
limit; it succeeds after release. No sustained spin or descriptor leak
appears in these runs.

Evidence: `s42-256.json`, `r{1,2,3}-s42-64.json` and matching logs.

## 5. Stage32 P0 cells

`chaos.py network --cells S2:P0,S3:P0,S4:P0,S7:P0`, three runs per cell:

| Cell | Result | Detail |
|---|---|---|
| S2-P0 | PASS 3/3 | QUIC 0.525–0.533 s, exact payload match |
| S3-P0 | PASS 3/3 | QUIC hop 0.791–0.838 s; TCP hop 1.133–1.204 s; exact payload matches |
| S4-P0 | PASS 3/3 | 50/50 per transport per run; maximum QUIC 0.014–0.015 s, TCP 0.007–0.008 s |
| S7-P0 | PASS 3/3 | all 10 selections per run chose QUIC |

Evidence: `chaos.log` and
`target/stage32-chaos/results/20261112T084329Z-network/`. Both campaign
builds identify commit `267385f`; certification daemon hash
`4560de44…`, lab-feature daemon hash `7364b662…`.

## 6. Local gates

`scripts/ci-local.sh` full: **13/13 PASS**, including the Python unit
tests, dependency audit, acceptance and all ten lab gates. Evidence:
`target/ci-local/20261112T091031Z/summary.tsv`, individual logs and
`target/stage49-m3/ci-local-recovery-host.log`.

## 7. Next

M3 criteria hold. M4 is the `KNOWN_LIMITATIONS` update and closure:
refused QUIC peers fail early where ICMP reaches the probe; filtered
ICMP and silent peers retain the 5 s window; the probe sends one extra
one-byte datagram and holds one extra socket per attempt. Forged ICMP
can fail an attempt early, as recorded in M2. No WAN check was run;
frozen release artifacts and the S3-P2/S2-P4 exclusions are unchanged.
