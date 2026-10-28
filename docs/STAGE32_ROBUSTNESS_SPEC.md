# Stage32 robustness and chaos certification — campaign spec

**Status: approved 2026-10-27 (X1–X5 as recommended, with the owner's
edits); step 2 (harness and dry run) done, see
[Step 2 notes](#step-2-notes).**

Stage32 changes no product behaviour. It adds a lab harness and
certification records; any defect it finds is fixed separately (section 7).

## 1. The sandbox (decision 1a)

Every campaign runs inside one sandbox, created without root:

- **Namespaces:** `unshare -rmnp --fork` (user, mount, network, PID). The
  invoking user is root inside and nowhere else. The sandbox's PID
  namespace contains every daemon, target and client, so ending its first
  process ends all of them, and nothing in it can see or signal any other
  process on the host (wolf-a, wolf-b, the Stage22R daemon).
- **A private root:** a new directory `/tmp/wwp-chaos-*` (mode 0755, owned
  by the invoking user, so owned by uid 0 inside) becomes `/` through
  `chroot`. It holds read-only bind mounts of `/usr`, `/lib`, `/lib64`,
  `/bin`, `/sbin` and `/etc`, its own `/proc`, a bind of `/dev`, the
  binaries at `/bins` (read-only) and the Dens under `/work/<node>` (0700).
  This is what lets the Den's ancestor check run **unchanged**: inside an
  unprivileged user namespace the host's root-owned directories appear
  owned by `nobody`, and the check (correctly) refuses them; under the
  private root every ancestor of a Den is owned by the sandbox's root.
  Verified by a probe on 2026-10-27: a daemon started, `pelt init`
  succeeded and both listeners were configured. The read-only remounts
  are verified in step 2.
- **Topology:** one network namespace per node, joined through a router
  namespace `R` by veth pairs, with forwarding enabled in `R` (verified by
  the same probe, with netem on a router link).

  | Namespace | Holds | Address |
  |---|---|---|
  | `A` | initiator daemon, test clients | 10.32.1.2 |
  | `B` | receiver daemon, echo and hash targets | 10.32.2.2 |
  | `Y` | relay daemon (Stage26 routed Fangs) | 10.32.3.2 |
  | `R` | router only | .1 on each link |

  Every daemon listens on its own address (TCP and QUIC on one port);
  control sockets live under `/work`. The sandbox has no route to the
  host network.
- **Impairing a link** means a netem qdisc on **both** ends of its veth
  pair (each end shapes its egress), with the same profile and seeds `s`
  (node end) and `s + 1` (router end). A campaign impairs the A–R link,
  the R–B link, or for routed Fangs the A–R and R–Y links, as the matrix
  says.
- **Two builds, each with its own provenance** (*owner edit*):
  - **the certification build:** `cargo build --release --locked
    --offline --workspace --bins --target-dir target/stage32-chaos`, with
    no features. Every run uses it except the one below, so what is
    certified is the release binary;
  - **the lab build:** the same with `--features werewolfd/lab-faults`
    and `--target-dir target/stage32-chaos-lab`, used **only** for the
    Stage31 automatic-sync item of S5, so it can set exact delays
    (decision X4).

  Each build writes its own provenance file: commit, `git status`,
  features, `rustc` and `cargo` versions, and the SHA-256 of `werewolfd`
  and `werewolfctl`. The host part is recorded once per campaign:
  `uname -r`, the `iproute2` and `libfaketime` versions. Every run records
  which build it used, and the records keep the two apart.

## 2. Network campaigns (decision 2a)

### Profiles

| Id | netem (per end) |
|---|---|
| P0 | none (baseline) |
| P1 | `loss 1%` |
| P2 | `loss 10%` |
| P3 | `loss gemodel 5% 15% 75% 0%` (25% of the time in a burst state losing 75%; about 19% overall) |
| P4 | `delay 50ms 20ms distribution normal` |
| P5 | `delay 200ms 100ms distribution normal` |
| P6 | `delay 10ms reorder 25% 50%` |
| P7 | `duplicate 5%` |
| P8 | `corrupt 1%` |
| P9 | `rate 1mbit limit 100` |

P4 and P5 also reorder packets as a side effect of jitter; that is
intended.

### Surfaces

| Id | What | Size or count |
|---|---|---|
| S1 | encrypted TCP v2 Fang, direct, simultaneous full-duplex echo, SHA-256 of both directions | 32 MiB (P9: 4 MiB) |
| S2 | QUIC Fang, direct, as S1 | 32 MiB (P9: 4 MiB) |
| S3 | routed Fang through `Y`, QUIC hop and TCP hop, as S1 | 16 MiB (P9: 2 MiB) |
| S4 | establishment: sequential Fang opens each followed by a 1 KiB echo, both transports | 50 per run |
| S5 | control plane: `discover refresh`, `discover services`, manual `governance sync` of a 4-statement chain, Stage31 automatic sync (lab delays) of a revocation | one of each per run |
| S6 | fences: Silver on `B`, and a Pack revoke of `A` on `B`, during an S1/S2 transfer; then a new open | one of each per run |
| S7 | selection: `secure` policy selection, with QUIC impaired | 10 selections per run |

### Matrix

| Surface | Profiles |
|---|---|
| S1, S2, S3 | P0–P9 |
| S4 | P0, P2, P3, P5, P7, P8 |
| S5 | P0, P2, P3, P5, P8 |
| S6, S7 | P0, P2, P3 |

47 cells (30 for S1–S3, 6 for S4, 5 for S5, 6 for S6 and S7), each run
3 times (decision 5a): 141 runs. (*Corrected by the owner:* the draft
said 168.)

## 3. Time campaigns (decision 3a)

### T1. Wall-clock jumps

One daemon at a time (`A`, then `B`, then `Y`) runs under
`LD_PRELOAD=libfaketime.so.1` with `FAKETIME_TIMESTAMP_FILE`,
`FAKETIME_NO_CACHE=1` and `FAKETIME_DONT_FAKE_MONOTONIC=1`. Its control
file steps through `+60s`, `-60s`, `+1h`, `-1h`, `+1d`, `-1d`, `+8d` and
`-8d` relative to real time:

- during an S2 transfer, and between S1 opens;
- with a Stage31 deadline pending (the acceptance check's monotonic test
  is repeated in every run: the deadline counts real seconds);
- with `discover refresh` from a peer after each jump.

Expected: transfers intact, opens succeed, sync applies as without
jumps, deadlines unmoved. The only expected change is to contact records:
after `+8d` the jumped node's own record is signed 8 days ahead, its peers
refuse it (`expires_at` more than 7 days ahead of their clock), and after
the jump back it is accepted again. That refusal is the documented limit,
not a failure; any other effect of a jump is a finding.

### T2. Stalls

`SIGSTOP` of `A`, `B` or `Y` for 30 s and for 300 s, once idle and once
during an S2 transfer (and an S1 transfer for the 300 s case), then
`SIGCONT`:

- the stalled daemon resumes and answers `status`;
- a transfer either continues and completes intact, or ends with an
  error (for 30 s both are possible, being at the QUIC idle timeout; for
  300 s it must end);
- new opens succeed after resume;
- an overdue Stage31 run happens once after resume (one new `last_run`,
  then a fresh interval), never a burst.

T1 and T2 each run 3 times per node: 9 + 9 runs.

## 4. PASS (for approval: survey section 4 made exact)

A run is PASS only if every applicable item holds. Anything the harness
cannot establish is **EXCLUDED**, reported with its cause, and never
counted as PASS (owner rule).

1. **Integrity.** Both directions' SHA-256 match exactly for every
   completed transfer and echo. Any mismatch is FAIL.
2. **Bounded liveness.** A transfer completes, or ends with an error,
   within its outer bound: 10 × its P0 time or 120 s, whichever is
   larger (P9: 900 s). An S4 open ends (success or error) within 20 s
   (`CLIENT_WINDOW` 15 s + 5 s). Exceeding a bound is FAIL. Completion
   times and S4 success counts are recorded, not asserted (decision X3).
3. **No downgrade.** No S7 selection, and no Fang under any profile, uses
   plain TCP. Any is FAIL.
4. **Fences hold, measured locally at `B`** (*owner edit*). Timed on
   `B`'s monotonic clock from the moment the Silver or revoke command
   returns on `B`: within 10 s (`CLEANUP_WINDOW` 5 s + 5 s), `B`'s
   inbound authority reports no active session and `B`'s side of the
   session (its target connection) is closed. A new open from `A` is then
   refused, within the 20 s open bound. When `A` sees the session end
   travels over the impaired link, so it is recorded, not asserted.
5. **No crash, no leak.** No daemon exits or panics. After each run and
   a 35 s settle (past the 30 s QUIC idle timeout), each daemon's open
   file descriptors are at most baseline + 4, threads at most baseline
   + 2, and resident memory at most baseline + 32 MiB (decision X2).
6. **Control plane.** Each sync applies exactly the expected sequences,
   each once (counted in the logs); a peer may be `unreachable`; a
   partial or duplicate apply is FAIL.
7. **Time (T1).** Every item above, plus: the monotonic deadline counts
   real seconds within 1.5 s; contact-record acceptance changes only as
   stated in section 3.

A scenario passes only if all 3 of its runs pass.

## 5. Reproduction and records

- Seeds: run `n` (1–3) of profile `Pk` uses `s = 1000·k + 10·n` on the
  node end and `s + 1` on the router end. netem's `seed` is set on every
  qdisc and recorded.
- Every run records: the exact `tc` and `faketime` commands, the build
  it used and that build's provenance,
  per-transfer bytes, hashes and times, the per-daemon fd/thread/RSS
  samples, and the result with its reason. Logs and the sandbox tree are
  kept for every FAIL or EXCLUDED run.
- Records: `docs/STAGE32_NETWORK_CERTIFICATION.md` and
  `docs/STAGE32_TIME_CERTIFICATION.md`, one table per surface, docs-only
  commits, as for Stage22R.

## 6. Out of scope

- The real WAN (decision 6a); Stage22R covers hard loss and blackholes
  there.
- The D09 indeterminate commit (decision 7a): stays unit-tested.
- IPv6 and other interfaces; real host clock changes (libfaketime shifts
  one process's view, not the kernel's); suspend and resume (the 300 s
  `SIGSTOP` stands in for a stall, not for a boot-clock gap).

## 7. Defects (for approval: survey section 8 made exact)

- A FAIL stops its campaign group. The failing run's evidence is kept and
  stays in the record: failures are never deleted or re-run away.
- The defect is fixed in its own PR, with a regression test, against
  `main`. No protocol, cryptographic, trust or downgrade semantics change
  as part of a robustness fix without a separate review.
- After the fix merges, every scenario of the affected surface is re-run
  (3 runs each) on the fixed build, and the record shows both builds.
- A harness defect (an EXCLUDED run) is fixed in the harness and the run
  repeated; the exclusion stays in the record.

## Spec decisions

All five approved by the owner on 2026-10-27.

- **X1. The outer bounds** of section 4 item 2: 10 × P0 or 120 s; P9
  900 s; opens 20 s. Recommended as written.
- **X2. The leak margins:** fds + 4, threads + 2, RSS + 32 MiB after a
  35 s settle. Recommended as written.
- **X3. S4 success counts recorded, not asserted.** Under 10% loss or
  corruption some opens may fail cleanly; what is asserted is no hang, no
  wrong bytes and no crash. Recommended.
- **X4. A separate lab build with `lab-faults`, for the S5
  automatic-sync item only,** so it can use exact Stage31 delays; the
  feature only reads lab environment variables. Every other run uses the
  certification build without features (section 1). Recommended.
- **X5. Sections 4 and 7** as the PASS and defect rules. Recommended.

## Implementation (after approval)

| Step | Scope | State |
|---|---|---|
| 1 | This spec | Done |
| 2 | Harness: sandbox, topology, netem and faketime control, sampling; dry run of P0 and P2 on S1 | Done (faketime control: step 4) |
| 3 | Network campaigns; `STAGE32_NETWORK_CERTIFICATION.md` | **Paused** at S1-P5 by the owner, see [Step 3 notes](#step-3-notes-interim). F1: fix in a separate PR; F2: documented limit; harness echo fixed; P5 payloads calibrated. Resumes when the owner says |
| 4 | Time campaigns; `STAGE32_TIME_CERTIFICATION.md` | |
| 5 | KNOWN_LIMITATIONS and STATUS updated to what ran | |

## Step 2 notes

The harness is `tests/integration/chaos.py`, with the transfer client
`tests/integration/chaos_client.py`. The dry run (S1, encrypted TCP direct,
32 MiB full duplex, the A–R link impaired on both ends) ran on the
certification build (`werewolfd` `266171be…cd7d`, no features), kernel
6.12.111, iproute2 6.15.0:

- **The sandbox:** the private root and `/work` are owned by the
  sandbox's root (uid 0), and writes to `/usr`, `/etc` and `/bins` fail
  as read-only. The daemons ran with the Den check unchanged.

| Profile | Run (seed) | Result | Time | Detail |
|---|---|---|---|---|
| P0 | 1 (10) | PASS | 1.31 s | 32 MiB both ways, hashes match |
| P0 | 2 (20) | PASS | 1.23 s | as above |
| P0 | 3 (30) | PASS | 1.30 s | as above |
| P2 | 1 (2010) | PASS | 82.9 s | hashes match (bound 120 s) |
| P2 | 2 (2020) | PASS | 59.7 s | hashes match |
| P2 | 3 (2030) | PASS | 8.07 s | **reset by the connection** after 1.9 MB sent, nothing echoed: a clean early end within the bound, which section 4 counts as PASS |

After every run and the 35 s settle, both daemons had 16 file
descriptors (baseline 16), 5 threads (baseline 6) and at most about
1 MiB more resident memory than at baseline.

- **Seeds reproduce runs.** P2 run 3 (seed 2030) ended the same way, at
  the same 8.07 s, in two separate dry runs.
- **Open question for step 3, not yet diagnosed:** why that connection
  was reset. The client no longer has any deadline but the outer bound,
  so the reset came from the Fang's side. One hypothesis, not a finding:
  the Fang's encrypted connection to `B` failed to establish, because
  TCP's retransmission backoff under 10% loss outlasted the 5 s handshake
  read window (`READ_WINDOW`). S4 (establishment under P2) is the
  campaign that measures this; the record will state the cause only if it
  is shown.
- **A harness defect, found and fixed before any evidence was kept.** In
  the first dry run, the client's receive loop set a 5 s timeout on the
  socket, which in Python also applied to the sender thread's `sendall`;
  under loss the send timed out and that run (P2, seed 2030) was judged
  PASS on the client's own error. The client is now a single
  non-blocking loop whose only deadline is the outer bound. That first
  dry run is not evidence and is not recorded above.

## Step 3 notes (interim)

The network campaign ran on 2026-10-27 (`main` at `6b96e98`, harness as in
this PR). Certification build `werewolfd` `266171be…cd7d`; lab build
(`lab-faults`, S5 automatic sync only) `2f5d2df8…3a40`; kernel 6.12.111,
iproute2 6.15.0. A P0 smoke test of every surface passed (21 of 21 runs)
before it.

**S1, encrypted TCP, direct, 32 MiB full duplex, A–R impaired** (evidence
retained, owner decision):

| Profile | Runs (seed: result, time) |
|---|---|
| P0 | 10, 20, 30: PASS, 1.2–1.4 s, hashes match |
| P1 | 1010, 1020, 1030: PASS, 1.6–2.5 s, hashes match |
| P2 | 2010: PASS 77.7 s; 2020: PASS 52.2 s (hashes match); 2030: PASS as a clean early end, reset at 8.06 s with nothing echoed (F1) |
| P3 | 3010, 3020, 3030: PASS as clean early ends, reset after 76.2 s, 27.1 s and 30.6 s with 24–74 KB echoed (F2) |
| P4 | 4010, 4020, 4030: PASS, 52.6–62.7 s, hashes match |
| P5 | harness verdict FAIL (two runs at the 120 s bound with 7.1 MB and 3.5 MB echoed; one reset at 24.4 s). **Reclassified by the owner as EXCLUDED: an invalid workload and bound, not a product failure** |

**A control on the same links and seeds** (raw kernel TCP from A to an echo
server on B's address, no Werewolf; diagnostic, not certification):

| Profile | Raw TCP |
|---|---|
| P2 | all three complete: 26.5 s, 29.2 s, 36.3 s; seed 2030 first echo after 3.07 s, then complete |
| P3 | none completes within 120 s (0.4–5.1 MB echoed); every connection stays up and progresses |
| P5 | none completes within 120 s (5.1–15.4 MB echoed): 32 MiB within 120 s is not attainable on this link, which is why S1-P5 is excluded |

**Owner decisions (2026-10-27):**

- S1 P0–P4: completed evidence retained.
- S1 P5: EXCLUDED, invalid workload and bound, not a product FAIL.
- **F1, OPEN:** an encrypted-TCP Fang fails to establish under P2 with
  seed 2030 (reset at about 8 s), where raw TCP stalls about 3 s and
  recovers. A setup robustness investigation.
- **F2, OPEN:** established encrypted-TCP Fang sessions end under P3
  (reset after about 24–76 s), where raw TCP stays connected and
  progresses. An established-session robustness investigation.
- The broad campaign is **paused**.

Also observed, not a finding by itself: under P2 the Fang's completed
transfers took about twice as long as raw TCP's.

### Diagnoses (2026-10-27)

Packet captures on A's link and loopback, with the same sandbox, profiles
and seeds (diagnostic, not certification). Evidence:
`target/stage32-chaos/results/20261027T165530Z-network/diagnostics/`.

- **F1.** A's SYN was lost and resent after 1 s; then A's 6-byte TLS
  ChangeCipherSpec was lost, and its retransmission, held back by the
  3 s minimum retransmission timeout that follows a lost SYN, came too
  late. A's own TLS had completed (under TLS 1.3 the initiator finishes
  first), so A was waiting for B's challenge. B closed the connection
  exactly 5.0 s after accepting it: the receiver bounds TLS accept, the
  challenge and the OPEN read by 5 s from TCP accept (`hs::READ_WINDOW`).
  Raw TCP has no such bound and recovered. With static neighbour entries
  the result was the same, so ARP is not the cause.
- **F2, two causes.** (1) A harness defect: the `targets.py` echo closes
  a connection after 10 s without data, and some campaign P3 resets were
  that target ending a stalled transfer. (2) With an echo that has no
  idle timeout, two of three P3 runs still reset, each 60 s after A's data
  last advanced (11.96 s → 72.17 s; 35.9 s → about 103 s): the
  encrypted-TCP `TCP_USER_TIMEOUT` of 60 s, the dead-peer bound. Under
  burst loss, retransmission backoff can leave data unacknowledged that
  long; raw kernel TCP keeps retrying for about 15 minutes.

### Owner decisions (2026-10-27, second)

- **F1: a robustness defect. Fix:** one setup retry from the initiator,
  TCP/TLS only, up to the first byte of the receiver's challenge; for the
  direct encrypted-TCP Fang and the encrypted-TCP leg of failover. The
  receiver's 5 s defence is unchanged. (Separate PR.)
- **F2: a documented architectural limit.** `TCP_USER_TIMEOUT` stays
  60 s; no product change. An encrypted-TCP session whose data stays
  unacknowledged for 60 s ends.
- **Harness: defect confirmed.** The data-carrying surfaces (S1–S4, S6)
  use `chaos_echo.py`, an echo with no idle timeout; S5 and S7 keep
  `targets.py` (no stalled data). The S1-P3 certification runs above are
  **EXCLUDED** (harness-affected).
- **P5: a spec and workload issue.** The S1-P5 evidence above is
  retained; S1–S3 under P5 are rerun with a calibrated workload (below).

### P5 calibration

The P5 payload for S1–S3 is the largest power-of-two MiB that raw kernel
TCP (no Werewolf) completes, full duplex, within half the 120 s bound for
all three of the cell's seeds (5010, 5020, 5030), on the same path: A to
B with the A–R link impaired (S1, S2) and A to Y with the A–R and Y–R
links impaired (S3, as the routed path crosses both). The harness probes
how much each seed echoes in 60 s, takes the largest power of two under
the least, and confirms it, halving until all three complete
(`chaos.py calibrate`). The bound stays 120 s.

After the echo change, a P0 smoke test of S1, S2, S3, S4 and S6 passed
(15 of 15 runs, `20261027T205030Z-network`; smoke, not evidence).

**Calibration run** (`20261027T203757Z-calibrate`, harness as in this PR;
raw TCP, so no Werewolf build is involved):

| Path | Probe: echoed in 60 s (5010, 5020, 5030) | Confirmed | Payload |
|---|---|---|---|
| S1, S2 (A–B) | 6.09 MB, 2.92 MB, 2.43 MB | 2 MiB: 44.5 s, 39.8 s, 36.9 s | **2 MiB** |
| S3 (A–Y) | 16 MiB in 38.0 s and 44.7 s; 10.9 MB | 8 MiB: 23.9 s, 52.4 s, 32.2 s | **8 MiB** |

Observed, not explained: the two-link path moved more than the one-link
path under the same profile. The rule takes each path as measured.

### F1 fix, validation (diagnostic)

200 sequential 1 KiB sessions through one encrypted-TCP Fang under P2
(seed 2030), three batches per build, with A's link captured:

| Build | Failed sessions | Of which | B's 5 s window expired |
|---|---|---|---|
| Before the fix (`266171be…cd7d`) | 6 of 600 | 3 first sessions at about 3.05 s, with no TCP packet captured (consistent with A's neighbour resolution failing under loss; not diagnosed further); 3 at about 6 s | 3, all before the challenge |
| With the fix (separate PR) | 1 of 600 | 1 at 6.03 s | 4: 3 before the challenge, each retried once and completed; 1 after the challenge, when A's OPEN was lost, which the fix does not retry |

The remaining failure is outside the retry boundary the owner set: the
receiver's 5 s window also bounds the OPEN read, and an OPEN lost after
the challenge ends the session.

### Campaign resumed (2026-10-27/28)

Resumed on `main` at `5a45ed9` (with the F1 fix, #81), certification
build rebuilt; results `20261027T231811Z-network`. S1-P0, P1, P2 and P4
kept from the first run (owner decision); S1-P3 and S1-P5 rerun.

| Cell | Result |
|---|---|
| S1-P3 | PASS: all three runs clean early ends, reset after 79.9 s, 90.9 s and 67.6 s (F2, documented limit) |
| S1-P5 | PASS: 2 MiB (calibrated), 38.8 s, 69.9 s, 69.6 s, hashes match |
| S1-P6 to S1-P9 | PASS |
| S2-P0 to S2-P2 | PASS |
| S2-P3 | PASS: all three runs clean early ends, QUIC reset after 51.0 s, 56.1 s and 33.8 s |
| S2-P4 | harness verdict FAIL: all three runs at the 120 s bound, still progressing (19.4–19.7 MB echoed of 32 MiB), daemons alive, no leaks. **F3**, reclassified by the owner (below) |

### F3: QUIC throughput under jitter (diagnosis)

Diagnostic only, on a separate build of `5a45ed9` with quinn statistics
and environment-set loss thresholds and windows (never shipped; patch
and hash with the evidence in
`20261027T231811Z-network/diagnostics/f3/`). 8 MiB transfers, seed 4010,
the A–R link impaired on both ends. Every transfer's hashes matched.

| Netem | QUIC | Encrypted TCP |
|---|---|---|
| 50 ms delay | 622 kB/s | 4448 kB/s |
| 50 ± 20 ms jitter, kept in order | 474 kB/s | 825 kB/s |
| 50 ± 20 ms jitter, reordered (P4) | 162 kB/s | 433 kB/s |

- **Reordering is read as loss.** P4 drops nothing (netem drops 0, no
  UDP errors, 1240 UDP sends from A and 1239 received at B), yet quinn
  declared 349 of 6778 packets lost on A and 415 on B, with 73–80
  congestion events, keeping the congestion window at 10–23 KB (124–350
  KB, no losses, without reordering). A's path MTU also fell from 1452
  to 1200 after a probe was wrongly declared lost. Turning off quinn's
  packet-count loss detection and widening its time threshold to 2 and
  4 RTT raised P4 throughput to about 196–245 and 290 kB/s; about 200
  losses were still declared at 4 RTT, a residue not explained.
- **A 64 KiB per-stream window.** B's QUIC stream receive window
  (`transport/quic.rs`, the resource bound) caps a stream at 64 KiB per
  round trip: QUIC throughput × RTT stayed at about 62 KB at 20, 50 and
  100 ms RTT. Wider windows brought QUIC to TCP's rate under pure delay
  (4707 kB/s at 50 ms) and changed nothing under P4.

### Owner decisions (2026-10-28)

- **S2-P4 (1a):** a documented limit, QUIC throughput under reordering
  is about a third of encrypted TCP's (spurious loss detection, no data
  lost). S2-P4 is **EXCLUDED**; the campaign resumes from S2-P5.
- **Loss detection (2a):** no change.
- **Stream window (3a):** documented: about 620 kB/s per QUIC stream at
  100 ms RTT, the cost of the 64 KiB memory bound.
- **Residual losses (4a):** the investigation is closed; the cause of
  the losses still declared at a 4 RTT threshold is recorded as
  unexplained.
