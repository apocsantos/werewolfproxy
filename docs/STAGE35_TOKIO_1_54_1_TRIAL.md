# Stage35 — tokio 1.54.1 remediation trial

**Result:** `STAGE35_TOKIO_1_54_1 = REJECTED` (Outcome B). The candidate
stopped at the S3-P2 regression gate. tokio stays at **1.52.3** on
`main`, and RUSTSEC-2026-0386 stays accepted as before. The candidate is
not certified.

Docs only. Nothing in `main`'s code or dependencies changes. The frozen
`v1.0.0-rc.2` tag, its archive and every earlier record are unchanged.

## Scope and constraints

This was a trial authorised by the owner on 2026-11-01. The only
production change was the tokio update. These stayed as they were:
quinn 0.11.12 and quinn-proto 0.11.18, the #92 and #93 remediations, the
QUIC windows and every other limit, the protocol, and the Stage34 rules
(kernel tail, F3, 120 s bound, 512 KiB payload, seed order). The trial
ran in a disposable worktree with sandbox daemons only; wolf-a and
wolf-b were untouched.

## Baseline

- `main` `7f89503a9df1522223bc7f978264bdba43ca6d49` (#99 merged);
  `STAGE34_S3_P2 = EXCLUDED`.
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18, rustls 0.23.46,
  rustls-webpki 0.103.16; `Cargo.lock` SHA-256 `a191a20e…d27b`.
- `cargo audit` 0.22.2, RustSec database commit `2295466c`
  (2026-10-30T12:12:39+01:00), 1395 advisories, 218 dependencies:
  0 vulnerabilities, 1 warning (RUSTSEC-2026-0386, tokio, accepted).

## Candidate

Branch `stage35/tokio-1.54.1`, kept locally and not pushed, in
`target/stage35-tokio/wt`:

- `62513c0` `fix(deps): tokio 1.54.1`, from
  `cargo update -p tokio --precise 1.54.1`. tokio moves from 1.52.3 to
  1.54.1, and no package is added, removed or changes version. Cargo
  also re-selected the `windows-sys` version on nine Windows-only edges
  (anstyle-query, anstyle-wincon, errno, nu-ansi-term, rustix,
  rustls-platform-verifier, socket2, tempfile, winapi-util), choosing
  among versions already locked (0.52.0, 0.60.2, 0.61.2). The Linux
  dependency tree of `werewolfd` and `werewolfctl` (`cargo tree
  --target x86_64-unknown-linux-gnu`, 188 entries) differs only in
  tokio. `Cargo.lock` SHA-256 `17b20892…aaac`.
- `2e58e0e` `test:` the Stage12 Quinn proof harness tokio pin, from
  `=1.52.3` to `=1.54.1` (test only, as #93 aligned the harness pins).
- Candidate `werewolfd`, certification build:
  `30daf680c9b08196ebeaba485d244df39a22d0f3f49c9d6b62b9a4b5c6ec9e42`.

## Gates

| Gate | Command | Result |
|---|---|---|
| Advisories | `cargo audit --no-fetch` (same database) | **PASS**: 0 vulnerabilities, 0 warnings, 218 dependencies; RUSTSEC-2026-0386 resolved, the #92 and #93 fixes intact |
| Format | `cargo fmt --check` | PASS |
| Lint | `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` | PASS |
| Rust tests | `cargo test --locked --offline --workspace` | PASS: 412 passed, 0 failed, 15 targets |
| Python unit tests | `python3 -B -m unittest discover -s tests/integration -p 'test_*.py'` | PASS: 64 of 64 |
| Acceptance | `python3 -B tests/integration/run.py` | PASS: 12 of 12 |
| Lab gates | `python3 -B tests/integration/<gate>.py` | all ten PASS: service 10, discovery 12, service_discovery 7, routed_fang 12, provider 7, governance 6, activation 4, sync 5, death 6, auto_sync 9 |
| Stage12 TLS 1.3 matrix | `cargo run --locked --offline [--release]` in `tests/stage12_tls_auth` | PASS: 8 of 8, debug and release |
| Stage12 Quinn proof | `cargo run --locked --offline --release` in `tests/stage12_quinn_auth` | COMPLETE: 3 of 3, UDP environment PASS |
| **S3-P2 regression** | `chaos.py network --cells S3:P2 --repeat 10` | **FAIL: STOP** (below) |
| Stage32 QUIC subset (S2-P0/P1/P2/P5, S3-P0, S4-P0/P2, S7-P2) | not run | stopped at the previous gate (rule 10) |

## S3-P2 on the candidate

Results are in `20261101T205725Z-network`, under
`target/stage35-tokio/wt/target/stage32-chaos/results/`. There were 30
runs: 27 PASS (with clean early ends), 2 EXCLUDED as kernel tails, and
**1 FAIL**. Every echoed byte of every run was in order, and all three
daemons were alive afterwards with no leaks.

**TCP hop**

| Run | Seed | Result | Bytes echoed of 524,288 | Last echo | Y's socket toward A (last sample) |
|---|---|---|---|---|---|
| 3 | 2030 | EXCLUDED, kernel tail | 519,400 | 44.9 s | `LAST-ACK`, send-q 10,585 |
| 12 | 2030 | EXCLUDED, kernel tail | 517,720 | 38.4 s | `LAST-ACK`, send-q 16,193 |
| **28** | **2010** | **FAIL**: liveness, outer bound | 458,384 | 77.9 s | **`CLOSE-WAIT`, send-q 69,872** |

The per-seed TCP-hop results:

| Seed | Completed | Median, max | At the bound | Clean early ends |
|---|---|---|---|---|
| 2010 | 9 of 10 | 20.0 s, 94.2 s | 1 | 0 |
| 2020 | 10 of 10 | 27.8 s, 81.3 s | 0 | 0 |
| 2030 | 6 of 10 | 14.6 s, 40.9 s | 2 (tails) | 2 |

**Why the trial stops.** Two of the handoff's rules trigger:

1. **Run 28 is a TCP failure that the kernel-tail rule does not cover.**
   Y's socket toward A was in `CLOSE-WAIT`, not application-closed. A
   had half-closed, Y no longer had a socket to B, and 69,872 bytes
   (roughly the 65,904 bytes A had not yet received, plus record
   framing) sat in Y's kernel send queue, while Y's daemon still held
   the socket open. The last echo was at 77.9 s, followed by 42 s of
   silence until the bound. One likely reading is that Y's daemon was
   still waiting for its last write to be accepted while the kernel
   retransmitted. The Stage34 capture saw the same `CLOSE-WAIT` shape
   on rc.2 with tokio 1.52.3 (`target/stage34-diag/seed2020-3`: 130 KB
   queued, then a reset at 84.8 s, within the bound). So the state
   itself is not new; what is new is a run that ends at the bound in
   it. Whether tokio 1.54.1 makes it more likely is **not
   demonstrated**.
2. **Two classified kernel tails in 30**, which is more than one, so
   the trial stops for owner review. For comparison at 512 KiB: the
   first Stage34 campaign had one tail-shaped FAIL (before the rule);
   the second had 0; the 10-round diagnostic cells had 1 (rc.2, tokio
   1.52.3) and 2 (pre-#93 `main`, tokio 1.52.3). The candidate's two
   are within that range, so the sample does not show a change in the
   tail rate either way.

**QUIC hop** (separate; the known Stage34 limitation): seeds 2010 and
2020 completed 20 of 20 (max 9.9 s); seed 2030 completed 4 of 10
(median 48.2 s, max 65.0 s) with 6 setup resets. This matches the
quinn-proto 0.11.18 behaviour recorded in Stage34 (rc.2: 7 of 10, 3
resets; bisection: 3 of 6, 3 resets), with no new QUIC failure kind.
S3-P2 stays EXCLUDED. This is not attributed to tokio.

## Decision

**REJECT the tokio 1.54.1 update as tested.** Under the trial's rules
the S3-P2 gate is a STOP, and the candidate cannot be shown free of a
TCP-hop regression.

- tokio stays at 1.52.3 on `main`. RUSTSEC-2026-0386 stays accepted on
  the same ground as before: neither Werewolf nor any shipped dependency
  other than tokio uses `AsyncFd` (Stage33 #93 record).
- Nothing is changed: no code, dependency, configuration, harness or
  rule, and no historical record.

## For the owner

These need a decision before any retry. None of them is taken here.

- Whether the `CLOSE-WAIT` stall (Y's daemon still holding the socket,
  with the remaining data already in Y's kernel queue) is the same
  kernel tail, which would need a change to the kernel-tail rule's
  criteria, or a separate finding. Either needs evidence first.
- Whether the comparison should be repeated: tokio 1.52.3 and 1.54.1
  builds alternating in the cell's own sequence, with captures. That
  would show whether run 28's state is more frequent on 1.54.1. A
  single 30-run campaign cannot separate a rate of about 3% from noise.
- A later tokio release can be tried under the same rules.

## Evidence

All of it is outside Git history, under `target/`:

- `target/stage35-tokio/logs/`: the fmt, clippy, test, unittest,
  acceptance, ten lab gate and Stage12 logs, and `s3p2.log`;
- `target/stage35-tokio/wt/target/stage32-chaos/results/20261101T205725Z-network/`:
  per-run JSON (`cells/S3-P2.json`, including Y's socket samples),
  daemon logs (`cells/S3-P2-work/`) and build provenance;
- `target/stage35-tokio/tree-before.txt`, `tree-after.txt` and
  `Cargo.lock.main`: the dependency-closure comparison;
- the candidate commits `62513c0` and `2e58e0e` on the local branch
  `stage35/tokio-1.54.1`.
