# Stage47 M2 — single-address dial without the race

**M2: PASS, O1 kept.** With a single address dialled directly, an
encrypted-TCP Fang out of descriptors costs the same CPU as before Stage46:
1.66 s mean against 1.67 s in five interleaved rounds (it was 1.82 s
against 1.63 s in M1). A QUIC Fang is unchanged. `KNOWN_LIMITATIONS.md`
now states the measured figures (R1). No wire, trust, Target Policy,
receiver or dependency change.

## 1. Change

| Decision | Where | Change |
|---|---|---|
| O1 (S47-D3) | `transport/dial.rs` `race` | with exactly one address, the attempt is awaited directly: no `JoinSet`, spawned task or `select!` loop. Same attempt, same error, same caller window. Every literal address (the certified lab and chaos paths, and any Pack address written as an IP) takes this path; a hostname with several addresses races as in Stage46 |
| R1 (S47-D1) | `KNOWN_LIMITATIONS.md`, Stage46 entry | the CPU item replaced with the measured figures (about 12% more per episode on encrypted TCP, none on QUIC, none in steady state), why Stage46 M3 overstated it, and that Stage47 removed it |

T1 (S47-D2) landed in M1.

## 2. Unit tests (1 new)

| Test | Shows |
|---|---|
| `dial::tests::a_single_address_returns_its_own_error_at_once` | one address: one attempt at 0 ms; a refusal (`ConnectionRefused`) and a reached peer's refusal (`PermissionDenied`) come back unchanged, within 50 ms |

The Stage46 dialer tests, including `a_single_address_is_one_attempt` and
`tcp_tries_the_next_address_when_the_first_is_refused`, pass unchanged.

## 3. A/B (`fang_flood.py fd:tcp fd:quic --nofile 64`, T1 metric)

Five rounds, each this build then the build before Stage46 M2
(`e897b31`, the M1 worktree), 2026-11-11 14:54–15:08 UTC. This build's
lab `werewolfd` `d226a528…` (the M1 commit plus the uncommitted O1 change,
as the lab's provenance records). Results in `target/stage47-m2/`.

| Scenario | Build | `cpu_seconds_flood` | mean |
|---|---|---|---|
| `fd:tcp` | O1 | 1.63, 1.64, 1.68, 1.72, 1.63 | **1.66** |
| `fd:tcp` | before M2 | 1.74, 1.61, 1.67, 1.64, 1.69 | **1.67** |
| `fd:quic` | O1 | 0.79, 0.86, 0.76, 0.79, 0.94 | 0.83 |
| `fd:quic` | before M2 | 0.80, 0.81, 0.82, 0.82, 0.83 | 0.82 |

M1, same method: `fd:tcp` 1.82 s (Stage46) against 1.63 s. The earlier
build's spread is wider this session (1.61–1.74 against 1.60–1.65); O1's
runs fall inside it and the means agree within 0.01 s. The criterion
(within the earlier build's spread) holds, so O1 is kept.

- B's log on this build is 1,774–1,828 bytes against 935: the two failure
  classes of Stage46 M2, each bounded at one line per 10 s.
- Every run: the daemon stayed up, recovered, and served a legitimate echo
  after the release.

**M1's reasoning was wrong.** It estimated the gap at about 0.27 ms per
race and judged a task spawn too cheap to explain it. Removing the task
machinery for the single-address case closed the gap, so it did. Why one
spawned task and `select!` per dial cost that much under descriptor
exhaustion was not investigated further.

## 4. Existing checks

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --all-features -D warnings`: PASS.
- `cargo test --workspace --all-features`: **454 passed**, 0 failed (453
  and the new test).
- `scripts/ci-local.sh` full (`target/ci-local/20261111T151438Z`): **13
  of 13 PASS**.

## 5. Next

M3 (survey §8): the Stage46 matrix ×1 (the multi-address race is
unchanged; literal addresses take the fast path); Stage42 floods ×3 with
T1; Stage32 S2, S3, S4 P0; `ci-local` full.
