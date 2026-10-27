# Stage32 robustness and chaos certification — survey

**Status: decisions accepted 2026-10-27 (1a, 2a, 3a, 5a, 6a, 7a as
recommended; see [Decisions](#decisions-accepted-2026-10-27)).** Docs only.
The PASS criteria (section 4) and defect handling (section 8) were not
ruled on separately; they go into the [spec](STAGE32_ROBUSTNESS_SPEC.md) for review with the exact
matrix, bounds and margins, as in Stage30 and Stage31.

This is the next item on the owner's order (after Stage31, before the CLI
architecture-complete candidate). [KNOWN_LIMITATIONS](../KNOWN_LIMITATIONS.md)
states what is still not run on any build:

- netem-style random loss, jitter and reordering;
- host clock changes;
- an indeterminate protected-state commit on real storage (unit-tested only,
  Stage22R D09).

Stage22R already certified, over the real WAN, restarts and hard crashes
(D01–D04), hard path loss and blackholes (D05A, D05B, D06), target failure
(D07), resource pressure (D08) and storage faults (D09). Stage32 does not
repeat those; it covers impaired-but-alive paths and time.

This survey answers eight questions:

1. where the campaigns run, and how they are isolated;
2. which network impairments, on which surfaces;
3. which time faults;
4. what PASS means;
5. how many runs, and how they are reproduced;
6. whether anything runs over the real WAN;
7. the indeterminate-commit gap;
8. what happens when a campaign finds a defect.

## What exists today (facts the decisions rest on)

- **Unprivileged network namespaces work on DarkSide** and need no root:
  `unshare -rn` gives a private loopback on which `tc qdisc ... netem`
  (delay, jitter, loss, reordering) and veth pairs work. A probe on
  2026-10-27 measured the expected RTT under 20 ms ± 5 ms delay, 1% loss
  and 10% reordering inside such a namespace. Nothing in it is visible to
  the host loopback, so wolf-a, wolf-b and the Stage22R daemon are never
  affected. (The `sch_netem` kernel module is loaded on DarkSide; whether
  the probe loaded it or it already was is not known.)
- **The wall clock reaches little.** By code inspection, `werewolfd` reads
  the wall clock (`SystemTime`) only for:
  - Stage24 contact records: signed `expires_at`, accepted only if in the
    future and at most 7 days ahead of the verifier (`MAX_LIFETIME_SECS`);
  - the random seed of Fang identifiers (no semantic effect).

  Handshake replay protection is bounded, in memory and time-free. Every
  timeout, rate limiter, read window and the Stage31 scheduler use the
  monotonic clock. The TLS verifier pins keys and ignores certificate
  validity times (`verify_server_cert` ignores `now`). A campaign should
  confirm this empirically rather than trust the inspection.
- **No tool for per-process wall-clock shifts is installed.** Linux time
  namespaces offset only the monotonic and boot clocks, not the wall
  clock. `libfaketime` is not installed. Changing the host clock would
  affect every process on the host, including the historical wolves.
- **Transport policy** must not change under impairment: a `secure`
  selection never falls back to plain TCP (the owner's "never silently
  downgrade a secure protocol").

## 1. Where, and how isolated

**1a. Disposable daemons in unprivileged network namespaces on DarkSide
(recommended).** Each campaign runs its daemons, targets and clients in
one or more `unshare -rn` namespaces joined by veth pairs, with netem on
the veth interfaces. No root, no change to the host network, nothing
shared with other processes. It extends the existing lab harness
(`tests/integration`), with the same build provenance and cleanup rules.

**1b. A root-owned `ip netns` setup.** Same isolation, but needs sudo for
every run; no gain over 1a.

**1c. netem on the real interface or `lo`.** Rejected: it would impair
the historical wolves and, on the VPS, production.

## 2. Network impairments and surfaces

Impairments (applied to one path, one direction or both, recorded per run):

| Profile | netem |
|---|---|
| light loss | 1% loss |
| heavy loss | 10% loss, and a 25% burst profile (Gilbert–Elliott) |
| delay and jitter | 50 ms ± 20 ms; 200 ms ± 100 ms (reorders as a side effect) |
| reordering | 25% reorder with 10 ms delay |
| duplication | 5% duplicate |
| corruption | 1% bit corruption |
| constrained rate | 1 Mbit/s with a small queue |

Surfaces:

- Fangs: encrypted TCP v2 and QUIC, direct and routed (Stage26 relay),
  with simultaneous full-duplex transfers and end-to-end hashes;
- session establishment (handshake and OPEN/ACK) under each profile;
- control-plane RPCs: discovery, `services`, `governance` sync and
  Stage31 automatic sync;
- authority fences under impairment: Silver and Pack revoke still close
  established sessions and refuse new ones;
- transport selection: under impairment the `secure` policy never
  selects plain TCP.

**2a (recommended):** all profiles on Fangs (both transports, direct and
routed); corruption, heavy loss and delay/jitter on establishment and on
the control plane; fences and selection under heavy loss.

**2b:** a reduced matrix (light/heavy loss and jitter only). Faster, but
leaves corruption and duplication, the two that probe integrity and replay
handling, untested.

## 3. Time faults

- **Wall-clock jumps on one node:** forward and backward by 1 minute,
  1 hour, 1 day and 8 days (past the contact-record window), mid-session
  and between sessions. Expected: sessions, Fangs, sync and the Stage31
  scheduler are unaffected; only contact-record acceptance changes, and
  records from a node skewed beyond 7 days are refused (a documented
  limit, not a failure).
- **Monotonic stalls:** `SIGSTOP` of a daemon (initiator, receiver, relay)
  for 30 s and 5 min, then `SIGCONT`. Expected: peers time out as
  specified, sessions fail cleanly or recover, the stalled node resumes
  without a crash, and its overdue timers (for example the Stage31
  scheduler) fire once, not in a burst.

How to shift the wall clock for one disposable daemon:

**3a. `libfaketime` (recommended).** The owner installs it on DarkSide
(`apt install faketime`, root, once); the campaign loads it with
`LD_PRELOAD` into disposable daemons only. It intercepts every wall-clock
read in the process, so it would also catch uses the inspection missed.
Jumps during a run use its control file.

**3b. A lab-only offset hook** in `unix_now()` under `lab-faults`. No
installation, but it only moves the code path already known to read the
clock, so it cannot find an unknown one.

**3c. A disposable VM** with real clock changes. Most faithful, much
heavier, and DarkSide has none set up.

## 4. What PASS means

A run passes only if all of these hold; anything ambiguous is excluded and
reported, never counted as PASS (owner rule):

1. **Integrity:** every byte delivered matches end to end (SHA-256 of both
   directions); corrupted or duplicated packets never reach the
   application.
2. **Bounded liveness:** a transfer either completes, or ends with a clean
   error within the documented timeouts (no hang past them). Completion
   time is recorded against the unimpaired baseline, not asserted, except
   for an outer bound per profile set in the spec.
3. **No downgrade:** transport selection under `secure` never yields plain
   TCP; no fallback beyond policy.
4. **Fences hold:** Silver and revoke close established sessions and
   refuse new ones under every profile.
5. **No crash, no leak:** no daemon exit or panic; file descriptors,
   threads and memory return to within a set margin of baseline after
   each campaign.
6. **Control plane:** sync and automatic sync either apply exactly the
   statements expected or report peers as unreachable; never a partial or
   duplicate apply.

## 5. Runs and reproduction

**5a (recommended):** each scenario 3 times; netem's `seed` is fixed per
run and recorded; every run records build provenance, the exact `tc`
commands, the binary hashes, and its raw results. A single FAIL fails the
scenario until explained.

**5b:** one run per scenario. Faster, but a flaky pass is
indistinguishable from a real one.

## 6. The real WAN

**6a. Lab only for Stage32 (recommended).** Impairing a real path needs
root on the interface and would impair production traffic on the VPS.
Stage22R covered hard loss and blackholes over the WAN; a netem campaign
does not need repeating there.

**6b. One WAN spot check** with netem on DarkSide's uplink, applied only to
the VPS address with a `tc` filter. Needs root and touches DarkSide's real
network; listed for completeness.

## 7. The indeterminate-commit gap (D09)

The rename-then-fsync window that yields `IndeterminateAfterRename` is
unit-tested by injection only.

**7a. Keep it unit-tested and say so (recommended).** Producing it on real
storage needs fault injection below the filesystem (for example a
dm-flakey device), which needs root and is out of proportion to what it
would add.

**7b. A root-run campaign with dm-flakey,** like `death_residue.py`: the
owner runs it once.

## 8. When a campaign finds a defect

Per the owner's rules: certification records are docs-only; a defect is
fixed in its own PR, with a regression test, and the affected campaign is
re-run on the fixed build. No protocol, cryptographic or trust semantics
change as part of a robustness fix without separate review.

## Plan (after decisions)

| Step | Scope |
|---|---|
| 0 | This survey; owner decisions 1–8 |
| 1 | Spec for review: harness, exact matrix, outer bounds, leak margins, record format |
| 2 | Harness: namespace topology, netem control, clock control; dry run |
| 3 | Network campaigns; record |
| 4 | Time campaigns; record |
| 5 | KNOWN_LIMITATIONS and STATUS updated to what was run |

## Recommendations in one line

1a, 2a, 3a (with SIGSTOP stalls), PASS as in section 4, 5a, 6a, 7a, and
section 8 as stated.

## Decisions accepted 2026-10-27

- **1a.** Disposable daemons in unprivileged network namespaces on
  DarkSide; nothing on the host loopback.
- **2a.** The full impairment matrix on both Fang transports, direct and
  routed, plus establishment, the control plane, fences under impairment
  and no downgrade.
- **3a.** `libfaketime` for per-daemon wall-clock jumps, with SIGSTOP
  stalls for the monotonic side. The owner installed it on DarkSide
  (`libfaketime` 0.9.10+2024-06-05+gba9ed5b2-0.6).
- **5a.** Each scenario 3 times, with fixed and recorded netem seeds.
- **6a.** Lab only for Stage32.
- **7a.** The D09 indeterminate commit stays unit-tested, and says so.

### 3a acceptance check (2026-10-27)

The owner's five-step check, run on the release build of `main`
(`werewolfd` `266171be…cd7d`) with one disposable daemon:

| Step | Result |
|---|---|
| 1. Start `werewolfd` under faketime at +1 day | Started under `LD_PRELOAD=libfaketime.so.1`, `FAKETIME_DONT_FAKE_MONOTONIC=1`, control file `+1d` |
| 2. A Werewolf-visible wall-clock value moves | The own contact record was signed with `seq` (its signing time) = real time + 86400 s, `expires_at` = `seq` + 7 days |
| 3. A monotonic deadline does not move | The Stage31 `next_due_in_secs` fell 600 → 595 over 5.0 real seconds |
| 4. Change the control value while running | Control file rewritten to `+9d` |
| 5. The running daemon observes the jump | The next `discover show` re-signed the record at real time + 777600 s (+9 days), `expires_at` moved by 8 days; the deadline fell 595 → 593 over 2.0 s across the jump and 593 → 588 over 5.0 s after it; the daemon kept running |

PASS, 8 of 8 checks. libfaketime shifts the monotonic clock too unless
`FAKETIME_DONT_FAKE_MONOTONIC=1` is set; step 3 is what shows the setting
took effect, so every time campaign keeps that check.
