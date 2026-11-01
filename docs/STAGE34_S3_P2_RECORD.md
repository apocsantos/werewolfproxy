# Stage34 — S3-P2 record: kernel tail, quinn-proto 0.11.18 regression

**Result:** `STAGE34_S3_P2 = EXCLUDED`. S3-P2 is not certified and must
not be represented as a PASS. Stage34 is closed with the findings below
(owner decision, 2026-11-01).

Docs only. No shipped code, dependency or configuration change in this
record. The [survey](STAGE34_S3_P2_RECALIBRATION_SURVEY.md) has the
decisions; the [Stage32 spec](STAGE32_ROBUSTNESS_SPEC.md) has the
calibration and kernel-tail sections.

## Owner decisions (2026-11-01)

- **quinn-proto stays at 0.11.18**, the security-remediated dependency
  of #93. No downgrade to a version with known unremediated advisories:
  0.11.17 lacks the fixes for RUSTSEC-2026-0340 and RUSTSEC-2026-0338
  (fixed in 0.11.18), and 0.11.15, the rc.1/pre-#93 version, lacks all
  five.
- **The QUIC send window stays unchanged** (`SEND_WINDOW` 512 KiB,
  `crates/werewolfd/src/transport/quic.rs`). The 4 MiB window below was a
  diagnostic build only and is not approved for production.
- **S3-P2 stays EXCLUDED**, with the affected conditions stated below.
- **The kernel-tail rule (#98) is kept as merged**, its acceptance
  criteria unchanged.

## What Stage34 did

| Step | Outcome |
|---|---|
| Survey #96 | decisions 1a–5a |
| Calibration #97 | S3-P2 payload 512 KiB (raw TCP, 30 of 30 within 60 s; the old 1 MiB missed in 10 of 30) |
| First 30 runs, `main` `90b31cb` (`20261101T120104Z-network`) | 29 PASS; run 14 (seed 2020) TCP hop at the bound |
| Capture, seed 2020 | the kernel retransmission tail on Y, independent of payload |
| Kernel-tail rule #98 | owner rule; EXCLUDED and counted, any other stall FAIL |
| Second 30 runs, `main` `8b8d521` (`20261101T143021Z-network`) | 29 PASS, 0 kernel tails; **run 27 (seed 2030) FAIL: the QUIC hop** at the bound |
| QUIC-hop diagnosis and bisection | the quinn-proto 0.11.18 regression below |

Both 30-run campaigns used the rc.2 production code (`werewolfd`
`fe66ba76…5c5`). Every echoed byte of every run was in order; no daemon
crashed or leaked.

**Run 27:** the routed QUIC hop reached the 120 s bound with 413,584 of
524,288 bytes echoed in order, first echo at 18.1 s, last at 119.9 s:
still progressing. The F3 rule covers a progressing routed QUIC hop only
under jitter and reordering (P4–P6), not under loss, so the harness
verdict is FAIL; it is not a kernel tail.

## The quinn-proto 0.11.18 regression

Diagnostic, not certification evidence. Evidence in
`target/stage34-diag/` (cell captures `cell-*`, UDP pcaps of A's and Y's
links, socket samples) and builds in `target/stage34-bisect/`.

**Reproduction method.** A fresh sandbox per run does not reproduce it:
20 fresh runs of seed 2030 (rc.2 and pre-#93 builds alternating) all
ended in a clean setup reset at 5–7 s. The slow transfers appear only
in the cell's own sequence (one sandbox, the three seeds in turn,
`chaos.transfers`), so the diagnosis ran the cell itself with captures
on every seed-2030 QUIC hop.

**Bisection** (each build: the cell, routed QUIC hop under P2):

| Build | quinn / quinn-proto | `werewolfd` | Rounds | Seed 2030 QUIC hop | Seeds 2010, 2020 |
|---|---|---|---|---|---|
| pre-#93 `main` `efb537d` | 0.11.9 / 0.11.15 | `38805ea7…675` | 10 | 10/10, median 5.3 s, max 7.3 s, 0 resets | 20/20, max 12.9 s |
| `efb537d` + proto 0.11.16 | 0.11.9 / 0.11.16 | `04f66fd3…9ba` | 6 | 6/6, median 4.8 s, max 6.1 s | 12/12 |
| `efb537d` + proto 0.11.17 | 0.11.9 / 0.11.17 | `7c9b8670…667` | 6 | 6/6, median 5.4 s, max 7.8 s | 12/12 |
| `efb537d` + proto 0.11.18 | 0.11.9 / 0.11.18 | `98dcfdbd…cb4` | 6 | **3/6, median 22.3 s, max 35.5 s, 3 resets** | 12/12 |
| **rc.2** (`e2ce899`) | 0.11.12 / 0.11.18 | `fe66ba76…5c5` | 10 | **7/10, median 28.8 s, max 73.9 s, 3 resets** | 19/20, 1 reset |
| rc.2 + proto 0.11.19 | 0.11.12 / 0.11.19 | `1e3a0295…752` | 6 | **3/6, median 25.0 s, max 27.1 s, 3 resets** | 12/12 |

The regression arrives with **quinn-proto 0.11.18** (independent of the
quinn crate) and is still present in **0.11.19**, the latest release on
2026-11-01. It shows on seed 2030's loss pattern; seeds 2010 and 2020
are unaffected.

**Packet captures.** In a slow rc.2 run (73.9 s), A sends nothing for
17.6 s after its first second, then moves the payload in small bursts
over about a minute; on the pre-#93 build the same run moves the 512 KiB
in 3 s. UDP segmentation batching is the same on both builds.

**Likely cause (not proven).** quinn-proto 0.11.18 changed send-buffer
accounting: the connection's send window now counts data that has been
acknowledged but is still retained behind an unacknowledged range
(`SendBuffer::buffered`, replacing `unacked`), where 0.11.17 released
acknowledged ranges immediately. Under heavy loss this can hold the
sender at its send window. A diagnostic build of rc.2 with a 4 MiB send
window (`29146635…a25`, not approved for production; 11 runs before it
was stopped) had no slow transfer, but all three of its seed-2030 runs
still ended in a setup reset at about 7 s, which no 0.11.15–0.11.17
build showed. The send window explains at most part of the regression.

## Affected conditions (S3-P2 EXCLUDED)

- **Surface:** S3, a Fang routed through one relay (`Y`), **QUIC hop**
  between A and Y.
- **Profile:** P2, `loss 10%` on both impaired links (A–R and Y–R), so
  the routed path crosses four 10% loss qdiscs.
- **Build:** quinn-proto 0.11.18 or 0.11.19 (rc.2 ships 0.11.18) with
  the 512 KiB send window.
- **Effect:** for some loss patterns (seed 2030), routed QUIC transfers
  of 512 KiB end at setup (clean reset, about half the runs) or crawl
  (20–120 s, once reaching the 120 s bound still progressing), where
  quinn-proto ≤ 0.11.17 completed in 4–8 s. No corruption, crash, leak,
  downgrade or security failure was observed.
- **TCP hop of the same cell:** the kernel retransmission tail (owner
  rule, #98) occurs on every build; it is not a regression.

Not affected, by the evidence available: the direct QUIC surface under
P2 (S2-P2 PASS on `e2ce899` in the #93 subset); S3-P2 seeds 2010 and
2020; every other Stage32 cell as certified on its recorded commit.

## Residual risks

- Routed QUIC throughput and session setup under heavy random loss are
  worse in rc.2 than with quinn-proto 0.11.15–0.11.17 (pre-#93 `main`).
  A routed QUIC Fang on a very lossy path may fail to establish or be
  much slower; encrypted TCP is the alternative the operator can select
  (`--transport tcp`), never an automatic fallback.
- The cause is not proven. Other loss patterns, rates or the direct
  surface may be affected in ways the seeds here did not exercise.
- S3-P2 cannot currently discriminate a later dependency change on the
  QUIC hop. A tokio update can still be compared on the S3-P2 TCP hop
  (FAILs and kernel-tail rate) against the two 30-run campaigns above,
  but not certified by S3-P2.
- The fresh-sandbox method does not reproduce the regression; any
  retest must use the cell's own sequence.

## Requirements for future retesting

S3-P2 may be retested, and only then certified, when one of these
holds:

1. a quinn-proto release later than 0.11.19 that keeps every RUSTSEC fix
   of 0.11.18 (cargo audit clean of the #93 advisories); or
2. a Werewolf QUIC transport configuration change, which needs the
   owner's architectural review first (the send window is a resource
   bound of the Werewolf admission model).

The retest:

- **Regression check first:** the cell with captures on seed 2030
  (`chaos.transfers` sequence, not fresh sandboxes), at least 6 rounds,
  against the 0.11.17 row above. Proceed only if seed 2030's QUIC hop
  matches it (no crawl, no excess setup resets).
- **Certification:** `chaos.py network --cells S3:P2 --repeat 10` on
  `main` with the candidate build: 30 runs, 512 KiB, no FAIL; the
  kernel-tail count stated and compared with this record's: 0 in the
  second 30 runs; in the first 30, before the rule, one TCP-hop FAIL of
  the tail's shape (uncaptured); 1 (rc.2) and 2 (pre-#93) in the
  10-round diagnostic cells.
- The kernel-tail rule, the F3 rule, the 120 s bound and the 512 KiB
  payload are unchanged unless the owner decides otherwise.
- Report upstream (quinn) with this record's reproduction, if the owner
  wants it; nothing has been reported.
