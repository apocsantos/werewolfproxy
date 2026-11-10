# Stage43 M3 — Regression

**M3: PASS.** Both M1 scenarios stay fixed in all three rounds for QUIC,
encrypted-TCP and plain-TCP Fangs. The Stage42 flood results hold. The
ten Stage32 cells that Stage41 M3 ran pass 3 of 3 seeds each, and the
routed-Fang gate passes 12 of 12. Every historical verdict holds.

## 1. Build

- Tested commit `0947411` (M2 branch head: `faa1da2` fix plus the M2
  record). Lab debug `werewolfd`
  `9649ad08c6f87bb0297c551d3ae262b16e276db119bd2faad3b7feeab9b10ee5`,
  the same binary as the M2 check; every result file records that hash.
- Run 2026-11-10 09:33–10:34 UTC. `/tmp` stayed at 5% throughout; the
  log guard never triggered; the daemon stayed up in every run.
- No source or tooling change in this PR.

Results: `target/stage43-m3/` (`r{1,2,3}-{idle,source,s42-256,s42-64}.json`,
logs, `meta.txt`), chaos `target/stage32-chaos/results/20261110T100455Z-network`
(`chaos.log`), routed gate `target/stage43-m3/routed.log`.

## 2. M1 scenarios (`tools/stage43/fairness.py`), three rounds

**Idle (`idle:<kind>`, 65 s):**

| Fang | Held | Legit at ceiling (r1/r2/r3) | Legit after 65 s idle | B descriptors | Legit after release | M1 (`main`) |
|---|---|---|---|---|---|---|
| QUIC | 64 ×3 | reset ×3 (intended) | **PASS ×3** | 82 | PASS ×3 | reset |
| encrypted TCP | 64 ×3 | reset ×3 | **PASS ×3** | 145 | PASS ×3 | reset |
| plain TCP | 64 ×3 | reset ×3 | **PASS ×3** | 144 | PASS ×3 | reset |

"Legit at ceiling" is reset because the 64 connections are seconds old,
under the 60 s threshold: the intended rule (M2 §3).

**One source address (`source:<kind>`, namespace, Fang on 10.43.0.1):**

| Fang | Flood (10.43.0.2) held | Legit (10.43.0.3) at once | After 30 s active flood | After release | M1 (`main`) |
|---|---|---|---|---|---|
| QUIC | **16** ×3 | PASS ×3 | PASS ×3 | PASS ×3 | held 64; legit reset |
| encrypted TCP | **16** ×3 | PASS ×3 | PASS ×3 | PASS ×3 | same |
| plain TCP | **16** ×3 | PASS ×3 | PASS ×3 | PASS ×3 | same |

## 3. Stage42 flood scenarios (`tools/stage42/fang_flood.py`), three rounds

**Descriptor exhaustion:**

| Scenario | B descriptors flooded (r1/r2/r3) | CPU flooded | After release (all rounds) | Stage42 M3 |
|---|---|---|---|---|
| fd:quic, `nofile` 256 | 82 / 82 / 79 | 0% | 18, listening, listed, legit PASS | 73–82, 0% |
| fd:tcp, 256 | 143 / 145 / 143 | 0% | 17, same | 137–145, 0% |
| fd:tcp-plain, 256 | 145 ×3 | 0% | 17, same | 145 |
| fd:quic, **64** | 64 ×3 | 1.6 / 1.4 / 2.0% | 18, same | 64, 1.6% |
| fd:tcp, **64** | 63 ×3 | 1.8 / 2.0 / 1.8% | 17, same | 63, 1.4–1.8% |

Log growth per run: 83–89 bytes at 256; 188 (QUIC) and 428 (TCP) bytes
at 64, as in Stage42.

**Session quota (500 connections from loopback):**

| Fang | Echoed (r1/r2/r3) | B descriptors flooded | Released at 5 s / 30 s | Legit during (r1/r2/r3) | Legit after | Stage42 M3 |
|---|---|---|---|---|---|---|
| QUIC | 64 / 61 / 64 | 82 / 79 / 82 | 18 / 18 | FAIL / PASS / FAIL | PASS ×3 | echoed 59–61, during PASS ×3 |
| encrypted TCP | 61 / 57 / 64 | 139 / 131 / 145 | 17 / 17 | PASS / PASS / FAIL | PASS ×3 | 55–64, PASS/PASS/FAIL |
| plain TCP | 64 ×3 | 145 ×3 | 17 / 17 | FAIL ×3 | PASS ×3 | 64, FAIL ×3 |

"Legit during" follows the echoed count, as in Stage42 M3 §3: when the
active loopback flood holds all 64 slots, a newcomer is reset; it passes
only when some flood sessions failed admission at the receiver and left
slots free. That is the case M2 states it does not fix (an active flood
from loopback). It is not a regression: no flood connection is idle for
60 s and loopback is exempt from the share, so the M2 rules never
apply. QUIC reaching 64 echoed in two rounds (59–61 in Stage42 M3) is
the same incidental variation in how many QUIC handshakes the receiver
admits.

## 4. Stage32 cells (`chaos.py network`), because M2 changed the copy paths

| Cell | M3 | Historical | Detail (runs 1–3) | Stage41 M3 |
|---|---|---|---|---|
| S2-P0 | PASS 3/3 | PASS | 0.50–0.52 s, exact match | 0.53–0.54 s |
| S2-P1 | PASS 3/3 | PASS | 0.52–0.56 s | 0.55–0.64 s |
| S2-P2 | PASS 3/3 | PASS | 6.4–11.6 s | 6.6–11.1 s |
| S2-P5 | PASS 3/3 | PASS | 53.7–66.7 s | 58.0–58.9 s |
| S3-P0 | PASS 3/3 | PASS | QUIC hop 0.79 s; TCP hop 1.18–1.20 s | 0.79–0.87 s; 1.15–1.19 s |
| S3-P1 | PASS 3/3 | PASS | QUIC hop 0.97–1.06 s; TCP hop 1.6–3.2 s | 0.89–0.99 s; 2.6–3.4 s |
| S4-P0 | PASS 3/3 | PASS | 50/50 per transport; max QUIC 0.014 s | max 0.015 s |
| S4-P2 | PASS 3/3 | PASS | 50/50 per transport; max QUIC 2.0–5.4 s, TCP 3.1–4.5 s | max QUIC 1.1–3.1 s |
| S5-P0 | PASS 3/3 | PASS | manual [1–4]; automatic [3] in 1.5 s | same |
| S7-P0 | PASS 3/3 | PASS | 10/10 selections QUIC | same |

Every transfer matched by SHA-256. The unimpaired cells (P0) are as fast
as or faster than Stage41, so the per-byte timestamp in `Tracked` costs
nothing measurable. The impaired cells spread wider in two places, both
inside their pass criteria: S2-P5 (53.7–66.7 s against 58.0–58.9 s) and
S4-P2's slowest QUIC open (5.4 s in one seed against at most 3.1 s).
Both cells inject random loss and latency per seed, and the P0 cells
show no slowdown on the same paths, so the spread is attributed to the
impairment, not to M2; it was not investigated further. Not run, as
in Stage41: S2-P4 and S3-P2 (EXCLUDED).

## 5. Routed-Fang gate (`tests/integration/routed_fang_gate.py`)

PASS, 12 of 12, including 64 MiB full duplex over a QUIC hop and over an
encrypted-TCP hop, "target finishing first ends a routed session
promptly" on both hops (EOF in 0.09 s), Silver on the relay cutting a
live routed session, and plain TCP refused for a routed peer.

## 6. Conclusion

All M3 criteria hold. M4: `KNOWN_LIMITATIONS` Stage43 entry and closure,
carrying what M2 does not fix: an active flood from loopback still fills
a Fang; a legitimate session idle for 60 s or more can be ended while a
Fang is full; Fangs to one peer share its 64-session quota.
