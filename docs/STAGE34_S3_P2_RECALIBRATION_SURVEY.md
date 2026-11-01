# Stage34 S3-P2 recalibration — survey

**Status: decisions accepted 2026-11-01, all as recommended (1a, 2a,
3a, 4a, 5a; see [Decisions](#decisions-accepted-2026-11-01)).**

The Stage32 cell **S3-P2** (routed Fang through `Y` under `loss 10%`,
QUIC hop then TCP hop) was certified 3 of 3 with a 1 MiB payload. The
Stage33 QUIC and webpki remediation showed that payload is marginal on
every build, and on the remediation commit S3-P2 is **EXCLUDED** (owner
decision; [record](STAGE33_QUIC_TLS_DEPS_REMEDIATION.md#s3-p2-on-the-final-commit),
[Stage32 note](STAGE32_ROBUSTNESS_SPEC.md#note-s3-p2-is-marginal-stage33-2026-10-31)).
S3-P2 is also the cell that showed the tokio 1.53.3 regression, so a
later tokio update cannot be judged until the cell is a reliable
discriminator again.

This survey answers five questions:

1. how S3-P2's workload is fixed;
2. how many runs certify it;
3. whether other calibrated cells are rechecked;
4. on which build;
5. what happens if the new workload still fails.

## What exists today (facts the decisions rest on)

- **The workload rule** (Stage32 spec, "P5 calibration" and "S3
  calibration"): the payload is the largest power-of-two KiB that raw
  kernel TCP, through a plain TCP relay on `Y` (`chaos_relay.py`, two TCP
  legs, as the TCP hop has), completes for all three of the cell's seeds
  within **half** the 120 s bound. Each seed is confirmed **once**
  (`chaos.py calibrate`). The bound stays 120 s (X1).
- **S3-P2's calibration** (`20261028T145605Z-calibrate`): probe
  3.5–4.8 MB echoed in 60 s; 2 MiB failed on seed 2030; 1 MiB confirmed
  in 24.1–26.5 s, three runs. The certified rerun
  (`20261028T151149Z-network`): QUIC hop 3.4–7.6 s, TCP hop 23.6–43.3 s.
- **The marginality** (Stage33 diagnosis, `target/stage33-diag/`, seeds
  2010/2020/2030 reproduced as the cell runs them):

  | Build | Runs | TCP hop at the 120 s bound |
  |---|---|---|
  | `main` `efb537d` (tokio 1.52.3) | 30 | 2 |
  | remediation `e2ce899` (tokio 1.52.3; rc.2 code) | 27 (with the campaign's) | 3 |
  | remediation with tokio 1.53.3 | 18 | 6 |

  Every run at the bound was still progressing with every echoed byte in
  order; no corruption, crash or leak. The cause in every captured case:
  `Y`'s daemon has relayed everything and closed, and `Y`'s kernel spends
  up to about 100 s retransmitting the last 60–130 KB toward `A`, its
  retransmission timer backed off to 17 s and 32 s after lost ACKs.
- **Werewolf's TCP hop is slower than the raw relay it was calibrated
  on.** Raw TCP confirmed 1 MiB in 24–27 s; completed Werewolf TCP-hop
  runs took 17–116 s (median about 45 s). Not explained; the calibration
  rule does not claim the two match, only that the payload is attainable
  on the path.
- **One confirmation per seed cannot see a tail of a few percent.** Three
  passing runs are consistent with a failure rate of 7% per run (the
  chance of three passes is about 80%).
- **The kernel tail does not shrink with the payload.** A smaller payload
  ends sooner and leaves more of the 120 s for the tail, but the
  retransmission backoff on the last segments can happen at any size.
  Whether a smaller payload removes the failures is to be measured, not
  assumed.
- **The other calibrated payloads**: S1/S2-P5 2 MiB and S3-P5 2 MiB (P5
  calibration), S3-P3 32 KiB. S1-P5/S2-P5 passed their campaign; S3-P5 is
  EXCLUDED under F3; every P3 cell is EXCLUDED as a documented limit. No
  marginality has been observed on S1-P5 or S2-P5, but none has been
  looked for beyond three runs.

## 1. The workload

- **1a. Recalibrate with a stricter confirmation (recommended).** Same
  rule, same path, same seeds and same 120 s bound, but the candidate
  payload is confirmed **10 times per seed** (30 raw-TCP runs), and every
  run must complete within half the bound; otherwise halve and repeat.
  Expected to land at 512 KiB or 256 KiB. `chaos.py calibrate` gains a
  repeat count; the routed P2 entry of `PAYLOAD_BY_PROFILE` takes the
  result.
- **1b. Keep 1 MiB, raise S3-P2's bound** (for example to 240 s). Changes
  the approved bound X1 for one cell; the observed tail (up to about
  100 s after a 20–110 s transfer) makes any fixed bound a guess.
- **1c. Calibrate against Werewolf itself** (the largest payload the
  Werewolf TCP hop completes). Circular: the cell would then measure
  only what it was tuned to pass. Not recommended.
- **1d. No change.** S3-P2 stays EXCLUDED as a documented marginal
  workload; later dependency updates are judged without it.

## 2. How many runs certify S3-P2

- **2a. 30 runs on the new payload, all PASS (recommended):** the three
  seeds × 10, each run as the cell runs it (QUIC hop then TCP hop), with
  the existing section 4 criteria (a clean early end within the bound is
  PASS). Zero failures in 30 bounds the per-run failure rate at about 10%
  (95%), against about 7% observed today; it is a discriminator, not a
  proof of zero.
- **2b. 3 runs, as Stage32.** Same as the original certification; does
  not address the problem that started this stage.
- **2c. 100 runs** (bound about 3%). About three hours of sandbox time;
  more confidence than a regression check needs.

## 3. Other calibrated cells

- **3a. S3-P2 only (recommended).** It is the only cell with observed
  marginality. The stricter confirmation is recorded as the rule for
  future calibrations; existing certified payloads stand as recorded.
- **3b. Recheck S1-P5, S2-P5 and S3-P5 under the stricter rule** (a raw
  calibration rerun, then 30 Werewolf runs per cell that changes). Wider
  evidence, roughly three times the work; S3-P5 would stay EXCLUDED
  under F3 regardless of payload.

## 4. Build

- **4a. `main` at the time of the run, rc.2 production code `e2ce899`
  (recommended).** The result becomes the S3-P2 baseline that a later
  tokio update is compared against. The rc.2 tag and archive are not
  touched; the Stage33 records stay as written.
- **4b. Also on `efb537d`** (the pre-remediation `main`), to show the
  new payload behaves the same on quinn 0.11.9. Not needed for the
  baseline.

## 5. If the new workload still fails

- **5a. Stop and report (recommended).** If any of the 30 runs FAILs on
  the recalibrated payload: one halving and a fresh 30 runs is allowed
  if the failing runs show the same kernel tail; anything else (a stall
  inside a daemon, corruption, a leak, or a second failure after
  halving) stops the stage for an owner decision under Stage32 section 8.
- **5b. Keep halving** down to the 1 KiB floor. Risks shrinking the cell
  until it tests nothing.

## Security notes

None. This is a test-harness workload and evidence count. No protocol,
trust, handshake, key, replay, fallback or shipped code change; Werewolf
binaries are built from `main` unchanged.

## Plan (after decisions)

| Step | Scope |
|---|---|
| 1 | This survey; decisions |
| 2 | Harness: calibration repeat count; raw-TCP calibration of S3-P2 (results `*-calibrate`); payload set from the result; its own PR |
| 3 | 30 S3-P2 runs on `main` (decision 2); record appended to the Stage32 spec, S3-P2 certified or stopped (decision 5); STATUS and KNOWN_LIMITATIONS updated |

The tokio update (RUSTSEC-2026-0386, now tokio 1.54.1) is a separate
remediation after this stage, judged with the new S3-P2 against this
baseline.

## Recommendations in one line

1a, 2a, 3a, 4a, 5a.

## Decisions accepted 2026-11-01

- **1a.** The S3-P2 payload is recalibrated by the existing rule with
  each seed confirmed 10 times (30 raw-TCP runs) within half the bound.
- **2a.** S3-P2 is certified by 30 Werewolf runs (three seeds × 10), all
  PASS.
- **3a.** S3-P2 only; the 10-fold confirmation is the rule for future
  calibrations, and existing certified payloads stand as recorded.
- **4a.** On `main`, rc.2 production code `e2ce899`.
- **5a.** One further halving if failures show the same kernel tail;
  anything else stops for an owner decision.
