# Stage32 robustness and chaos certification — campaign spec

**Status: draft for review.** No campaign has run. This implements the
accepted decisions of the [Stage32 survey](STAGE32_ROBUSTNESS_SURVEY.md#decisions-accepted-2026-10-27)
(1a, 2a, 3a, 5a, 6a, 7a) and puts the PASS criteria (survey section 4) and
defect handling (survey section 8), which were not ruled on separately,
up for approval here with exact numbers.

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
| 1 | This spec | |
| 2 | Harness: sandbox, topology, netem and faketime control, sampling; dry run of P0 and P2 on S1 | |
| 3 | Network campaigns; `STAGE32_NETWORK_CERTIFICATION.md` | |
| 4 | Time campaigns; `STAGE32_TIME_CERTIFICATION.md` | |
| 5 | KNOWN_LIMITATIONS and STATUS updated to what ran | |
