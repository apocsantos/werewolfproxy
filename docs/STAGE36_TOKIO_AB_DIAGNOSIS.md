# Stage36 — tokio 1.52.3 vs 1.54.1, controlled A/B diagnosis

**Result.** TCP: **NO MATERIAL REGRESSION DETECTED**, and the Stage35
run-28 pattern reproduced on tokio 1.52.3. QUIC: **NO MATERIAL REGRESSION
DETECTED**; the timing difference seen with instrumentation did not
persist without it. tokio 1.54.1: **ACCEPT FOR FURTHER CERTIFICATION**,
which needs a separate Stage37 decision first. Nothing is promoted.

Diagnosis only (owner authorisation, 2026-11-01). tokio stays at
**1.52.3** on `main`, with RUSTSEC-2026-0386 accepted. No code,
dependency, configuration, harness or rule changed. The frozen
`v1.0.0-rc.2` tag and archive are untouched. The Stage34 and Stage35
records and verdicts stand as written; the Stage35 report is in
PR #100, still open, and its run-28 FAIL is unchanged. wolf-a and wolf-b
were not touched.

## 1. Question and method

The question: does tokio 1.54.1 materially worsen routed encrypted-TCP
reliability, or does it expose an interaction that tokio 1.52.3 already
has?

- **Cell:** S3-P2 exactly as `chaos.transfers` runs it. One sandbox
  runs seeds 2010, 2020 and 2030 in turn, 5 rounds per block, with a
  512 KiB payload and the 120 s bound. The kernel-tail and F3 rules are
  unchanged. Each run is the QUIC-hop leg, then the TCP-hop leg. The
  driver is `target/stage36/s3_ab_capture.py`, which calls the
  harness's own `transfers`.
- **Instrumented campaign:** block order A, B, B, A (2026-11-01
  21:54–23:19 UTC). On every TCP-hop leg it ran a headers-only `tcpdump`
  on A's and Y's links and `ss -tinmoH` on A and Y every second. The
  QUIC legs were not captured.
- **Uninstrumented campaign:** block order B, A, A, B (2026-11-01 23:20
  to 2026-11-02 07:01 UTC), with no capture. The harness's own
  per-second sampling of Y's sockets, part of the kernel-tail rule, ran
  in both campaigns.
- **Totals:** 30 runs per build per campaign, so 120 runs. The actual
  start time of every leg is recorded (`order` in each `cell.json`).
- **Host suspend:** in the uninstrumented block 3 (build A), the host
  suspended from about 00:08:27 to 06:26:05 UTC. The user journal is
  silent for that span, then desktop services wake together. The
  suspend fell inside run 6, between its QUIC leg and its TCP leg. Runs
  6–15 of that block ran after a resume. They are reported, flagged, and
  also excluded in a sensitivity check.

## 2. Builds

Both builds come from identical Werewolf source (`main` `7f89503`). The
only difference is the tokio lockfile change and the Stage12 harness pin
(`git diff 7f89503 2e58e0e`). Both were built with the same command:
`cargo build --release --locked --offline --workspace --bins`, each with
its own target directory.

| | Build A | Build B |
|---|---|---|
| Source | `7f89503` | `2e58e0e` (Stage35 candidate: `62513c0` + `2e58e0e`) |
| tokio | 1.52.3 | 1.54.1 |
| `Cargo.lock` SHA-256 | `a191a20e…d27b` | `17b20892…aaac` |
| `werewolfd` | `fe66ba76d55111a0fb837f4c4f0ceb13a25c6c2dd6b28a96a41df07fe1aa05c5` (byte-identical to the rc.2 certification build) | `30daf680c9b08196ebeaba485d244df39a22d0f3f49c9d6b62b9a4b5c6ec9e42` (byte-identical to the Stage35 candidate) |
| `werewolfctl` | `24de9943…3d22` | `41cd9f4f…92da` |

Both use quinn 0.11.12, quinn-proto 0.11.18, rustls 0.23.46 and
rustls-webpki 0.103.16. Their Linux dependency closures (`cargo tree
--target x86_64-unknown-linux-gnu`, 188 entries) differ only in tokio
(`target/stage36/closure-{a,b}.txt`).

## 3. Results

Rates carry Wilson 95% intervals. "Shift" is the Hodges–Lehmann B−A
estimate with a seed-stratified bootstrap 95% interval, and Cliff's
delta is reported for B against A. p values come from a
seed-stratified permutation test of ranks, with labels permuted within
each seed. Full output is in `target/stage36/stats.txt`.

### A. Instrumented: A against B

| | A: tokio 1.52.3 | B: tokio 1.54.1 |
|---|---|---|
| TCP completed | 26/30 | 26/30 |
| TCP FAIL | 0 [0–11%] | 0 [0–11%] |
| Kernel tails (EXCLUDED) | 1 [1–17%] | 1 [1–17%] |
| TCP clean early ends (reset) | 3 | 3 |
| `CLOSE-WAIT` of 10 s or more | 5 [7–34%] | 10 [19–51%], Fisher p 0.23 |
| TCP median / p90 / p95 / max | 25.6 / 53.2 / 83.7 / 107.4 s | 21.4 / 49.1 / 82.6 / 86.7 s |
| TCP shift B−A | | −4.9 s [−9.8, +0.9], Cliff −0.25, p 0.08 |
| QUIC 2010+2020 median / p90 / max | 2.3 / 6.5 / 14.8 s | 4.6 / 8.8 / 11.7 s |
| QUIC 2010+2020 shift | | **+1.4 s [+0.2, +3.6], Cliff +0.40, p 0.03** |
| QUIC 2030 completed, setup resets | 6/10, 4 | 5/10, 5 |
| Integrity failures, dead daemons, leaks | 0 | 0 |

### B. Uninstrumented: A against B

| | A: tokio 1.52.3 | B: tokio 1.54.1 |
|---|---|---|
| TCP completed | 23/30 | 26/30 |
| **TCP FAIL** | **2 [2–21%]**: `CLOSE-WAIT` at the bound | 0 [0–11%], Fisher p 0.49 |
| Kernel tails (EXCLUDED) | 1 | 0 |
| TCP clean early ends | 4 | 4 |
| TCP median / p90 / p95 / max | 28.1 / 49.1 / 53.9 / 55.1 s | 19.0 / 58.5 / 64.1 / 106.2 s |
| TCP shift B−A | | −3.8 s [−10.2, +1.8], Cliff −0.18, p 0.80 |
| QUIC 2010+2020 median / p90 / max | 2.3 / 6.2 / 7.8 s | 2.7 / 5.6 / 7.5 s |
| QUIC 2010+2020 shift | | +0.4 s [−0.5, +1.2], Cliff +0.17, p 0.35 |
| QUIC 2030 completed, setup resets | 7/10, 3 | 4/10, 6 |
| Integrity failures, dead daemons, leaks | 0 | 0 |

Sensitivity check: without the 10 runs that followed the suspend, A has
20 runs, with 1 FAIL (block 2, run 10, before the suspend), 0 kernel
tails and a TCP median of 26.0 s.

Per block (TCP median; QUIC 2010+2020 median):

- instrumented: A1 24.0, 1.9 s; B2 21.3, 4.7 s; B3 21.4, 4.2 s; A4 26.9,
  2.5 s;
- uninstrumented: B1 19.8, 2.6 s; A2 21.6, 2.4 s; A3 29.5, 2.2 s; B4
  18.2, 2.9 s.

The per-seed TCP and QUIC distributions are in `stats.txt`. No per-seed
TCP shift has p below 0.13.

### C. Instrumentation effect, within each build

| | Build A: instrumented against uninstrumented | Build B: instrumented against uninstrumented |
|---|---|---|
| TCP shift | −1.3 s [−7.1, +4.9], p 0.51 | −1.0 s [−5.9, +4.2], p 0.86 |
| Kernel tails | 1 against 1 | 1 against 0 |
| TCP FAIL | 0 against 2 | 0 against 0 |
| QUIC 2010+2020 shift | +0.1 s [−0.8, +0.7], p 0.94 | **−1.3 s [−3.3, −0.0], p 0.055** |
| QUIC setup resets | 4 against 3 | 5 against 6 |

Within Stage36, instrumentation did not measurably change TCP completion
times or tail counts on either build. The difference between
instrumented A and the earlier uninstrumented Stage34 campaign
(p 0.03) does not appear inside this controlled design, so it is better
explained by campaign and time differences than by the instrumentation.
`CLOSE-WAIT` durations can only be measured with the instrumentation,
so the effect on them cannot be compared. The one notable within-build
difference is build B's QUIC hop, which ran slower in the instrumented
campaign. QUIC legs ran with no capture of their own, so a direct
mechanism is not evident.

## 4. The `CLOSE-WAIT` stall

**Where it comes from in the code.** On Y, an encrypted-TCP relay
session runs `secure_copy_server_side` (`transport/tcp_encrypted.rs`).
When B's side ends (`read` returns 0), Y calls `shutdown()` on its
encrypted stream toward A. Before that, `write_encrypted_frame` must
hand every remaining frame to the socket. A writing future suspends
while the socket is not writable. Y's socket toward A is in
`CLOSE-WAIT` because A has half-closed after sending its 512 KiB, and Y
has not yet shut down its side.

**Instrumented evidence, both builds.** All 15 `CLOSE-WAIT` intervals of
10 s or more, 5 on A and 10 on B, had Y's queued bytes above two-thirds
of the socket's send buffer (`skmem` `w` against `tb`) in **every**
per-second sample. That is Linux's writability threshold
(`sk_stream_is_writeable`), so in none of the 15 intervals was the
socket ever sampled writable. The queue drained only as A's
acknowledgements arrived. Thirteen intervals ended in `LAST-ACK`, which
means Y completed its writes and shut down, and two (one on each build)
lasted to the end of their legs. The earlier tokio 1.52.3 capture shows the same thing
(Stage34, `target/stage34-diag/seed2020-3`): 72 s in `CLOSE-WAIT`, the
queue above the send buffer throughout, `bytes_acked` frozen for about
60 s and backoff up to 4. Per-interval data is in
`target/stage36/closewait-instrumented.txt`.

**Run-28 pattern.** A `CLOSE-WAIT` stall that reaches the bound
happened **on tokio 1.52.3**, twice, in the uninstrumented campaign:

| Build | Block, run | Seed | Bytes echoed | Last echo | Y toward A at the last sample |
|---|---|---|---|---|---|
| A, 1.52.3 | uninstr. block 2, run 10 | 2010 | 229,112 | 78.5 s | `CLOSE-WAIT`, send-q 134,664 |
| A, 1.52.3 | uninstr. block 3, run 12 (after the suspend) | 2030 | 479,920 | 92.9 s | `CLOSE-WAIT`, send-q 62,264 |
| B, 1.54.1 | Stage35, run 28 (for comparison) | 2010 | 458,384 | 77.9 s | `CLOSE-WAIT`, send-q 69,872 |

None occurred in the 60 Stage36 runs of build B. Both builds show the
pattern, and 1.52.3, the production version, showed it more often in
this sample (2 in 30 uninstrumented, 1 in 20 without the post-suspend
runs).

**Interpretation, and its limits.** The evidence on both builds fits
this picture: the relay application is blocked on a socket that is not
writable, and progress is limited by TCP retransmission backoff toward
A. This is the same kernel interaction as the kernel tail, but caught
before the relay can shut down. The limits:

- Sampling once a second cannot rule out brief writable moments between
  samples.
- A non-empty send queue does not by itself prove that the application
  has forwarded everything.
- The two uninstrumented FAILs on A carry only the harness's
  last-sample socket state, not a timeline.
- The application's internal task state was not traced. No
  instrumentation was added to Werewolf code, so "blocked writing" is
  inferred from socket state, not observed.

No evidence points to a tokio wakeup defect. The Stage34 kernel-tail
rule is **not** extended to `CLOSE-WAIT`; such runs remain FAIL under
the existing rules.

## 5. QUIC

The QUIC observations are kept separate from TCP. S3-P2 stays EXCLUDED
for QUIC certification because of quinn-proto 0.11.18 (Stage34).

- **Seeds 2010 and 2020.** With instrumentation, B was slower (median
  2.3 s against 4.6 s; shift +1.4 s [+0.2, +3.6], p 0.03). Without it,
  the difference did not persist: 2.3 s against 2.7 s, shift +0.4 s
  [−0.5, +1.2], p 0.35. Splitting each run, time to first echo (setup
  plus first round trip) was the same in both campaigns: medians 0.4
  against 0.5 s instrumented, 0.6 against 0.6 s uninstrumented. The
  instrumented difference lay in the transfer after the first echo
  (+0.9 s, p 0.11). Given the many comparisons in this record, a
  p of 0.03 that does not replicate is consistent with chance or with a
  campaign-level effect. It is not evidence of a tokio effect.
- **Seed 2030**, the known quinn-proto limitation: completions 6/10 and
  7/10 on A against 5/10 and 4/10 on B; setup resets 4 and 3 against 5
  and 6. Completed transfers were faster on B in the uninstrumented
  campaign. All of this is within the Stage34 description, with no new
  QUIC failure kind (no QUIC hop reached the bound). Nothing here is
  attributed to tokio.

## 6. Classification

- **TCP: NO MATERIAL REGRESSION DETECTED.** Neither campaign shows
  tokio 1.54.1 with more TCP FAILs, kernel tails, clean early ends or
  longer completions. The point estimates favour 1.54.1 slightly, but
  not significantly. The Stage35 run-28 pattern reproduced on 1.52.3,
  so it is an existing kernel and application interaction, not a 1.54.1
  regression. Limits: 30 runs per build per condition bound the FAIL
  rate only to roughly 0–11% (0 of 30), and a difference of a few
  percent cannot be detected. This is not proof of equivalence.
- **QUIC: NO MATERIAL REGRESSION DETECTED** within the sensitivity of
  the test. The instrumented signal did not replicate without
  instrumentation, and setup time was unchanged in both campaigns. QUIC
  certification remains EXCLUDED for the independent quinn-proto
  reason.
- **A separate observation, not a tokio finding:** a routed
  encrypted-TCP session can reach the bound with the relay blocked on a
  non-writable socket in `CLOSE-WAIT`. It is present on the production
  tokio 1.52.3. The current rules count it as FAIL, so S3-P2's TCP hop
  cannot reliably pass the all-PASS standard on **either** tokio
  version.

## 7. Recommendations

1. **tokio 1.54.1: ACCEPT FOR FURTHER CERTIFICATION.** That requires a
   separate **Stage37** owner decision; it is not promoted by this
   record. The Stage35 advisory result (0 vulnerabilities, 0 warnings)
   holds only for that candidate and database (`2295466c`).
2. Stage37 must first decide how S3-P2's TCP hop is judged, given that
   the `CLOSE-WAIT`-at-bound FAIL occurs on 1.52.3. Two possibilities
   for the owner:
   - a comparative criterion: the candidate's FAIL and tail rates
     against a concurrent 1.52.3 control, run in blocks alternating
     between builds as here;
   - a separate investigation of the relay's shutdown under
     backpressure, with lifecycle tracing in a diagnostic build only.

   Either needs architectural review. No rule is changed here.
3. Until Stage37 decides: tokio 1.52.3 stays, RUSTSEC-2026-0386 stays
   accepted, and the Stage35 REJECTED verdict stands as the record of
   that trial under its own rules.
4. No rc.3.

## 8. Limitations and open questions

- The samples are small: 30 runs per build per condition, with three
  seeds of 10 each. Rate intervals are wide.
- The host suspended during uninstrumented block 3 (build A). Its
  post-resume runs are flagged, and the sensitivity check excludes them.
- The two campaigns ran at different times of day. Block order was
  alternated within each campaign and reversed between them, but the
  QUIC timing in particular varied between campaigns.
- Socket evidence is sampled once a second. The application's task
  state is inferred, not traced.
- The tokio version affects only the Werewolf daemons. The Python
  harness, its client and its echo are the same for both builds.

## 9. Evidence

All of it is outside Git history, under `target/stage36/`:

- `build-{a,b}/` (binaries) and `closure-{a,b}.txt`;
- `runs/blk*` (instrumented: `cell.json`, per-run pcaps
  `run*-tcp-link-{a,y}.pcap`, `run*-tcp-ss.json`, daemon logs) and
  `runs-nocap/blk*` (uninstrumented);
- `per-run.csv` (SHA-256 `4c7f7865…a647`), `stats.txt`,
  `closewait-instrumented.txt`, and the scripts `s3_ab_capture.py`,
  `ab_stats.py`, `ab_analyze.py`, `campaign.sh` and
  `campaign-nocap.sh`;
- Stage34 comparison: `target/stage34-diag/seed2020-3`.

## Appendix: per-run results

"bound" means the leg reached the 120 s bound. "reset" is a clean early
end. The Y column shows the last sample of Y's socket toward A for TCP
legs that did not complete. Run numbers are within each block; seeds
follow the harness order 2010, 2020, 2030.

| Cond. | Block | Run | Seed | Result | QUIC hop | TCP hop | Y toward A, last sample |
|---|---|---|---|---|---|---|---|
| instr | blk1-a | 1 | 2010 | PASS | 2.992 s ok | 19.167 s ok |  |
| instr | blk1-a | 2 | 2020 | PASS | 1.967 s ok | 107.367 s ok |  |
| instr | blk1-a | 3 | 2030 | PASS | 8.895 s reset | 13.982 s ok |  |
| instr | blk1-a | 4 | 2010 | PASS | 1.894 s ok | 22.573 s ok |  |
| instr | blk1-a | 5 | 2020 | PASS | 3.669 s ok | 46.555 s ok |  |
| instr | blk1-a | 6 | 2030 | PASS | 20.805 s ok | 27.732 s ok |  |
| instr | blk1-a | 7 | 2010 | PASS | 1.507 s ok | 23.106 s ok |  |
| instr | blk1-a | 8 | 2020 | PASS | 4.19 s ok | 83.712 s ok |  |
| instr | blk1-a | 9 | 2030 | PASS | 28.227 s ok | 20.854 s ok |  |
| instr | blk1-a | 10 | 2010 | PASS | 1.5 s ok | 7.043 s reset | FIN-WAIT-2 0 |
| instr | blk1-a | 11 | 2020 | PASS | 1.289 s ok | 23.885 s ok |  |
| instr | blk1-a | 12 | 2030 | PASS | 43.013 s ok | 26.943 s ok |  |
| instr | blk1-a | 13 | 2010 | PASS | 2.084 s ok | 17.415 s ok |  |
| instr | blk1-a | 14 | 2020 | PASS | 1.33 s ok | 24.167 s ok |  |
| instr | blk1-a | 15 | 2030 | PASS | 38.813 s ok | 51.27 s ok |  |
| instr | blk2-b | 1 | 2010 | PASS | 2.166 s ok | 23.497 s ok |  |
| instr | blk2-b | 2 | 2020 | PASS | 7.816 s ok | 42.278 s ok |  |
| instr | blk2-b | 3 | 2030 | PASS | 76.499 s reset | 23.894 s ok |  |
| instr | blk2-b | 4 | 2010 | PASS | 5.207 s ok | 13.707 s ok |  |
| instr | blk2-b | 5 | 2020 | PASS | 6.653 s ok | 15.602 s ok |  |
| instr | blk2-b | 6 | 2030 | PASS | 37.504 s ok | 18.999 s ok |  |
| instr | blk2-b | 7 | 2010 | PASS | 7.287 s ok | 14.274 s ok |  |
| instr | blk2-b | 8 | 2020 | PASS | 1.676 s ok | 42.14 s ok |  |
| instr | blk2-b | 9 | 2030 | PASS | 9.106 s ok | 7.471 s ok |  |
| instr | blk2-b | 10 | 2010 | PASS | 1.57 s ok | 28.094 s ok |  |
| instr | blk2-b | 11 | 2020 | PASS | 2.654 s ok | 30.935 s ok |  |
| instr | blk2-b | 12 | 2030 | PASS | 12.623 s reset | 19.18 s ok |  |
| instr | blk2-b | 13 | 2010 | PASS | 8.838 s ok | 17.02 s ok |  |
| instr | blk2-b | 14 | 2020 | PASS | 4.267 s ok | 86.673 s ok |  |
| instr | blk2-b | 15 | 2030 | PASS | 27.512 s ok | 104.582 s reset | LAST-ACK 33605 |
| instr | blk3-b | 1 | 2010 | PASS | 8.337 s ok | 6.225 s reset | FIN-WAIT-1 25 |
| instr | blk3-b | 2 | 2020 | PASS | 2.94 s ok | 43.292 s ok |  |
| instr | blk3-b | 3 | 2030 | PASS | 12.627 s reset | 28.087 s ok |  |
| instr | blk3-b | 4 | 2010 | PASS | 3.598 s ok | 49.124 s ok |  |
| instr | blk3-b | 5 | 2020 | EXCLUDED | 11.732 s ok | 120.001 s bound 507384 B | LAST-ACK 28615 |
| instr | blk3-b | 6 | 2030 | PASS | 7.471 s reset | 13.081 s ok |  |
| instr | blk3-b | 7 | 2010 | PASS | 10.209 s ok | 17.719 s ok |  |
| instr | blk3-b | 8 | 2020 | PASS | 4.847 s ok | 21.819 s ok |  |
| instr | blk3-b | 9 | 2030 | PASS | 8.902 s reset | 10.92 s ok |  |
| instr | blk3-b | 10 | 2010 | PASS | 2.879 s ok | 15.798 s ok |  |
| instr | blk3-b | 11 | 2020 | PASS | 5.46 s ok | 6.033 s reset | FIN-WAIT-1 1 |
| instr | blk3-b | 12 | 2030 | PASS | 66.014 s ok | 14.294 s ok |  |
| instr | blk3-b | 13 | 2010 | PASS | 2.931 s ok | 21.072 s ok |  |
| instr | blk3-b | 14 | 2020 | PASS | 1.31 s ok | 28.56 s ok |  |
| instr | blk3-b | 15 | 2030 | PASS | 24.261 s ok | 82.583 s ok |  |
| instr | blk4-a | 1 | 2010 | PASS | 2.273 s ok | 26.794 s ok |  |
| instr | blk4-a | 2 | 2020 | PASS | 5.699 s ok | 30.832 s ok |  |
| instr | blk4-a | 3 | 2030 | PASS | 43.172 s reset | 53.164 s ok |  |
| instr | blk4-a | 4 | 2010 | PASS | 2.534 s ok | 52.023 s ok |  |
| instr | blk4-a | 5 | 2020 | PASS | 2.486 s ok | 24.472 s ok |  |
| instr | blk4-a | 6 | 2030 | PASS | 56.85 s ok | 27.551 s ok |  |
| instr | blk4-a | 7 | 2010 | PASS | 14.85 s ok | 26.972 s ok |  |
| instr | blk4-a | 8 | 2020 | PASS | 1.811 s ok | 87.527 s reset | LAST-ACK 75435 |
| instr | blk4-a | 9 | 2030 | PASS | 12.624 s reset | 19.574 s ok |  |
| instr | blk4-a | 10 | 2010 | PASS | 6.5 s ok | 22.0 s ok |  |
| instr | blk4-a | 11 | 2020 | PASS | 7.492 s ok | 97.286 s reset | LAST-ACK 19221 |
| instr | blk4-a | 12 | 2030 | PASS | 8.88 s reset | 21.364 s ok |  |
| instr | blk4-a | 13 | 2010 | PASS | 1.862 s ok | 34.402 s ok |  |
| instr | blk4-a | 14 | 2020 | PASS | 2.373 s ok | 15.443 s ok |  |
| instr | blk4-a | 15 | 2030 | EXCLUDED | 48.158 s ok | 120.001 s bound 523320 B | LAST-ACK 7171 |
| uninstr | blk1-b | 1 | 2010 | PASS | 7.131 s ok | 6.457 s reset | ESTAB 0 |
| uninstr | blk1-b | 2 | 2020 | PASS | 7.515 s ok | 20.979 s ok |  |
| uninstr | blk1-b | 3 | 2030 | PASS | 8.911 s reset | 14.044 s ok |  |
| uninstr | blk1-b | 4 | 2010 | PASS | 3.695 s ok | 17.01 s ok |  |
| uninstr | blk1-b | 5 | 2020 | PASS | 2.629 s ok | 16.091 s ok |  |
| uninstr | blk1-b | 6 | 2030 | PASS | 7.006 s reset | 19.942 s ok |  |
| uninstr | blk1-b | 7 | 2010 | PASS | 2.645 s ok | 9.711 s ok |  |
| uninstr | blk1-b | 8 | 2020 | PASS | 2.561 s ok | 22.237 s ok |  |
| uninstr | blk1-b | 9 | 2030 | PASS | 7.006 s reset | 17.461 s ok |  |
| uninstr | blk1-b | 10 | 2010 | PASS | 1.157 s ok | 46.13 s ok |  |
| uninstr | blk1-b | 11 | 2020 | PASS | 1.604 s ok | 40.683 s ok |  |
| uninstr | blk1-b | 12 | 2030 | PASS | 20.192 s ok | 13.606 s ok |  |
| uninstr | blk1-b | 13 | 2010 | PASS | 1.598 s ok | 40.793 s ok |  |
| uninstr | blk1-b | 14 | 2020 | PASS | 3.594 s ok | 106.491 s reset | LAST-ACK 20501 |
| uninstr | blk1-b | 15 | 2030 | PASS | 47.445 s ok | 19.762 s ok |  |
| uninstr | blk2-a | 1 | 2010 | PASS | 1.445 s ok | 29.187 s ok |  |
| uninstr | blk2-a | 2 | 2020 | PASS | 7.835 s ok | 38.869 s ok |  |
| uninstr | blk2-a | 3 | 2030 | PASS | 37.087 s ok | 9.241 s ok |  |
| uninstr | blk2-a | 4 | 2010 | PASS | 1.431 s ok | 34.81 s ok |  |
| uninstr | blk2-a | 5 | 2020 | PASS | 6.255 s ok | 22.119 s ok |  |
| uninstr | blk2-a | 6 | 2030 | PASS | 43.172 s reset | 13.352 s ok |  |
| uninstr | blk2-a | 7 | 2010 | PASS | 2.047 s ok | 49.132 s ok |  |
| uninstr | blk2-a | 8 | 2020 | PASS | 1.091 s ok | 35.699 s ok |  |
| uninstr | blk2-a | 9 | 2030 | PASS | 52.38 s ok | 15.235 s ok |  |
| uninstr | blk2-a | 10 | 2010 | FAIL | 5.17 s ok | 120.001 s bound 229112 B | CLOSE-WAIT 134664 |
| uninstr | blk2-a | 11 | 2020 | PASS | 2.392 s ok | 119.528 s reset | CLOSE-WAIT 121632 |
| uninstr | blk2-a | 12 | 2030 | PASS | 12.681 s reset | 15.672 s ok |  |
| uninstr | blk2-a | 13 | 2010 | PASS | 2.456 s ok | 21.037 s ok |  |
| uninstr | blk2-a | 14 | 2020 | PASS | 2.595 s ok | 83.561 s reset | LAST-ACK 39271 |
| uninstr | blk2-a | 15 | 2030 | PASS | 59.984 s ok | 16.05 s ok |  |
| uninstr | blk3-a | 1 | 2010 | PASS | 3.15 s ok | 40.266 s ok |  |
| uninstr | blk3-a | 2 | 2020 | PASS | 2.063 s ok | 5.003 s reset | ESTAB 0 |
| uninstr | blk3-a | 3 | 2030 | PASS | 53.177 s ok | 53.871 s ok |  |
| uninstr | blk3-a | 4 | 2010 | PASS | 1.621 s ok | 28.127 s ok |  |
| uninstr | blk3-a | 5 | 2020 | PASS | 2.205 s ok | 23.809 s ok |  |
| uninstr | blk3-a (after suspend) | 6 | 2030 | PASS | 8.387 s reset | 6.014 s reset | ESTAB 0 |
| uninstr | blk3-a (after suspend) | 7 | 2010 | PASS | 4.039 s ok | 29.464 s ok |  |
| uninstr | blk3-a (after suspend) | 8 | 2020 | PASS | 1.88 s ok | 18.68 s ok |  |
| uninstr | blk3-a (after suspend) | 9 | 2030 | PASS | 69.518 s ok | 18.421 s ok |  |
| uninstr | blk3-a (after suspend) | 10 | 2010 | PASS | 2.91 s ok | 30.529 s ok |  |
| uninstr | blk3-a (after suspend) | 11 | 2020 | PASS | 2.208 s ok | 21.583 s ok |  |
| uninstr | blk3-a (after suspend) | 12 | 2030 | FAIL | 56.761 s ok | 120.001 s bound 479920 B | CLOSE-WAIT 62264 |
| uninstr | blk3-a (after suspend) | 13 | 2010 | PASS | 2.204 s ok | 35.051 s ok |  |
| uninstr | blk3-a (after suspend) | 14 | 2020 | PASS | 6.221 s ok | 55.063 s ok |  |
| uninstr | blk3-a (after suspend) | 15 | 2030 | EXCLUDED | 36.494 s ok | 120.002 s bound 502320 B | LAST-ACK 36703 |
| uninstr | blk4-b | 1 | 2010 | PASS | 5.065 s ok | 17.583 s ok |  |
| uninstr | blk4-b | 2 | 2020 | PASS | 4.683 s ok | 45.035 s ok |  |
| uninstr | blk4-b | 3 | 2030 | PASS | 11.41 s reset | 6.038 s reset | ESTAB 0 |
| uninstr | blk4-b | 4 | 2010 | PASS | 2.602 s ok | 64.061 s ok |  |
| uninstr | blk4-b | 5 | 2020 | PASS | 2.182 s ok | 15.096 s ok |  |
| uninstr | blk4-b | 6 | 2030 | PASS | 14.241 s ok | 25.548 s ok |  |
| uninstr | blk4-b | 7 | 2010 | PASS | 1.662 s ok | 34.402 s ok |  |
| uninstr | blk4-b | 8 | 2020 | PASS | 2.664 s ok | 106.165 s ok |  |
| uninstr | blk4-b | 9 | 2030 | PASS | 7.003 s reset | 18.236 s ok |  |
| uninstr | blk4-b | 10 | 2010 | PASS | 1.759 s ok | 14.352 s ok |  |
| uninstr | blk4-b | 11 | 2020 | PASS | 3.724 s ok | 15.154 s ok |  |
| uninstr | blk4-b | 12 | 2030 | PASS | 38.518 s ok | 11.15 s ok |  |
| uninstr | blk4-b | 13 | 2010 | PASS | 5.645 s ok | 7.035 s reset | FIN-WAIT-1 1 |
| uninstr | blk4-b | 14 | 2020 | PASS | 3.199 s ok | 58.464 s ok |  |
| uninstr | blk4-b | 15 | 2030 | PASS | 53.051 s reset | 11.894 s ok |  |
