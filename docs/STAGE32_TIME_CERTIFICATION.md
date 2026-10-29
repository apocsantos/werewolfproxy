# Stage32 time certification

The time campaigns of `docs/STAGE32_ROBUSTNESS_SPEC.md` section 3 (step 4):
T1, wall-clock jumps, and T2, stalls. Harness: `tests/integration/chaos.py
time`, in the same unprivileged sandbox as the network campaign, no netem.

## T1. Wall-clock jumps (2026-10-29)

**Setup.** A, B and Y, each in its own namespace, all three in each other's
Pack, each advertising its QUIC address. Governance: a genesis, an admit of
all three and an activation, applied on each, so the Stage31 automatic sync
is eligible with a deadline pending on every node. One node (A, B or Y,
three runs each) runs under `LD_PRELOAD=libfaketime.so.1` with
`FAKETIME_TIMESTAMP_FILE`, `FAKETIME_NO_CACHE=1` and
`FAKETIME_DONT_FAKE_MONOTONIC=1`; the others on real time. Its control file
steps through `+60s`, `-60s`, `+1h`, `-1h`, `+1d`, `-1d`, `+8d`, `-8d`
(relative to real time) and back to `+0s`. At each step:

- two S1 opens (encrypted TCP, 1 KiB echo) before the jump and two after;
- a 512 MiB S2 transfer (QUIC) in flight across the jump; when Y is the
  jumped node, the transfer and the opens go through Y (QUIC and TCP hops);
- the jumped node's Stage31 deadline (`next_due_in_secs`) read before the
  jump and at least 5 real seconds later: it must fall by the real seconds
  elapsed, within 1.5 s (spec section 4, item 7). When an automatic run
  happens in between, the deadline is new and the sample is recorded as
  skipped;
- `discover show` on the jumped node, then `discover refresh` from each
  peer: whether the peer now holds the jumped node's current record;
- a new statement applied at B, then `governance sync` from B on A and on
  Y: each must apply exactly that sequence, once.

### First run: a finding at -8d

`20261029T122337Z-time` (T1:A only, harness before the owner decision
below), harness verdict FAIL, all 3 runs on one point: at `-8d` both peers
refused A's record. Everything else held in every step.

A's clock 8 days behind signs a record with `expires_at` = A's now + 7 days,
which is 1 day before real time: the peers see an expired record. The same
lifetime check as at `+8d` (`expires_at` must lie within the verifier's
`[now, now + 7 d]`, `MAX_LIFETIME_SECS`), from the other side.

| Offset | A's record, relative to real time | Peers |
|---|---|---|
| `+8d` | `seq` +8 d, `expires_at` +15 d | refuse (too far ahead) |
| `-8d` | `seq` +8 d, `expires_at` −1 d | refuse (expired) |
| back to `+0s` | `seq` +8 d, `expires_at` +7 d | accept |

**Owner decision (2026-10-29):** the `-8d` refusal is the same documented
limit. A node whose clock is more than 7 days off real time, in either
direction, cannot publish a contact record its peers accept; it is
accepted again once its clock is back. The T1 expectation is: refused at
`+8d` and `-8d`, accepted at every other step and after the jump back.

Also observed, not a refusal: after `+8d` the record's `seq` stays about
8 days ahead even back on real time (`seq` only increases, by design);
peers accept it.

### Certification

`20261029T125839Z-time`, `main` at `22cee5c` plus the T1 harness
(`1e3bc0d`); certification build `werewolfd` `6333a482…042c`; kernel
6.12.113, libfaketime 0.9.10+2024-06-05+gba9ed5b2-0.6.

| Jumped node | Runs | Transfers (512 MiB, across the jump) | Deadline error | Contact records |
|---|---|---|---|---|
| A | 3 of 3 PASS | 27 of 27 intact, 7.8–8.2 s | −0.73 to +0.71 s (3 samples skipped: automatic run) | refused at ±8d, accepted otherwise |
| B | 3 of 3 PASS | 27 of 27 intact, 7.8–9.0 s | −0.58 to +0.68 s (3 skipped) | refused at ±8d, accepted otherwise |
| Y | 3 of 3 PASS | 27 of 27 intact through Y, 24.3–25.2 s | −0.86 to +0.22 s (3 skipped) | refused at ±8d, accepted otherwise |

Every S1 open succeeded (324 of 324); every sync applied exactly its one
new sequence on A and on Y (162 of 162); daemons alive and within the leak
margins after every run.

**T1 is certified (9 of 9).**

## T2. Stalls with SIGSTOP and SIGCONT (2026-10-29)

**Setup.** The same three nodes and governance as T1, on real time. Two
Fangs from A to B: S1 (encrypted TCP) and S2 (QUIC); when Y is the stopped
node both are routed via Y. One node (A, B or Y, three runs each) is
stopped with `SIGSTOP` on the `werewolfd` process itself and resumed with
`SIGCONT`. Each run has four phases:

- **idle 30 s** and **idle 300 s**: no transfer in flight. A governance
  statement is applied at a peer before the stop so that the next
  automatic sync has something to apply, and the stop begins while that
  sync is due within 15 s, so it falls due during the pause.
- **30 s during transfers**: a 512 MiB S2 transfer in flight across the
  stop.
- **300 s during transfers**: 512 MiB on S2 and on S1 in flight across the
  stop.

After every resume: `status` must answer; two new S1 opens (1 KiB echo)
must succeed with matching hashes. A transfer stopped for 30 s may
complete intact or end; one stopped for 300 s must end; no transfer may
reach the outer bound (420 s), and none may complete with a hash
mismatch.

**Automatic sync.** The certification build's schedule (first run 5–30 s,
interval 450–1350 s) cannot be made to fall due inside a pause
deterministically, so this check uses a lab-build twin of the stopped node
(`werewolfd` built with `lab-faults`, `acc56df9…a211`), in the same
namespace on its own port with its own lab peer, a first run after 1 s and
a fixed 60 s interval (`WEREWOLF_LAB_AUTO_SYNC_FIRST_MS`,
`WEREWOLF_LAB_AUTO_SYNC_INTERVAL_MS`). It is stopped and resumed together
with the certification node. After resume its status is polled every
0.2 s for 20 s. PASS: exactly one run starts, within 5 s of resume, it
applies the new statement, and the next run is due a fresh interval later
(60 ± 2.5 s). Two or more runs is a burst and FAILs. The scheduler is the
same code in both builds; only the timings come from the environment.

### Certification

`20261029T132555Z-time`, `main` at `22cee5c` plus the T1 harness and
record (`1e3bc0d`, `c0706f0`) and the T2 harness (`138e1ef`);
certification build `werewolfd` `6333a482…042c`, lab build
`acc56df9…a211`; kernel 6.12.113. The provenance records the commit as
`c0706f0` with `chaos.py` modified: the run started from the T2 harness
before it was committed as `138e1ef`, with identical content.

| Stopped node | Runs | Overdue automatic sync, both pauses | 30 s during transfers (S2) | 300 s during transfers (S2, S1) |
|---|---|---|---|---|
| A | 3 of 3 PASS | one run 0.2 s after resume, next due in 59 s; head as expected | 2 completed intact (38.0 s), 1 ended (reset, 31.0 s) | all ended at resume (reset, 301.0 s) |
| B | 3 of 3 PASS | the same | 1 completed intact (52.2 s), 2 ended (reset, 31.0 s) | all ended during the pause: S2 at 31.0–41.0 s, S1 at 61.3–61.5 s |
| Y | 3 of 3 PASS | the same | 1 completed intact through Y (76.6 s), 2 ended (reset, 30.9 s) | all ended during the pause: S2 at 30.9–41.0 s, S1 at 61.3 s |

`status` answered within 4 ms after every resume (36 of 36); every new
open succeeded (72 of 72); no transfer reached the outer bound and none
completed with a mismatch; daemons alive after every run.

When the stopped node is B or Y, the sessions end during the pause, at
times consistent with QUIC's 30 s idle timeout (`QUIC_IDLE_TIMEOUT`) and
encrypted TCP's 60 s `TCP_USER_TIMEOUT`. When A, the client's own node, is
stopped, the far side ends them the same way and A sees the reset as soon
as it resumes.

**T2 is certified (9 of 9).** With T1, the Stage32 time campaign is
complete.
