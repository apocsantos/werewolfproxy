# Stage48 M2 — failover does not redial a provider that did not answer

**M2: PASS.** In failover's encrypted-TCP leg, a TCP connect that timed
out is no longer retried, so the next provider is tried at once. One
silent encrypted-TCP provider now hands over at 5.02–5.04 s (was 10.04–10.06 s),
two reach the third at 10.03–10.05 s (were reset at 15.0 s), the same as
QUIC providers. The direct Fang keeps every Stage32 F1 retry and is
unchanged at 10.0 s (P3, recorded by owner decision S48-D2). No wire,
trust, Target Policy, receiver or dependency change.

## 1. Change (O1, S48-D1)

| Where | Change |
|---|---|
| `transport/tcp_encrypted.rs` | `connect_tcp_session` takes a `SetupRetry`: `BeforeChallenge` (every F1 case, as before) or `NotAConnectTimeout` (the same, except a TCP connect that ended in the 5 s connect timeout, `io::ErrorKind::TimedOut`, is final). The choice applies only to the connect step: a TLS transport failure and a close or silence before the challenge are retried in both |
| `transport/tcp_encrypted.rs`, direct Fang | `BeforeChallenge`: unchanged |
| `transport/failover.rs`, encrypted-TCP leg | `NotAConnectTimeout`: the next provider is the retry |

A refused or unreachable connect fails in milliseconds and is still
retried on both paths; it costs nothing (M1).

## 2. Unit tests (2 new)

| Test | Shows |
|---|---|
| `setup_retry_tests::failover_still_retries_a_close_before_the_challenge` | with `NotAConnectTimeout`, a TCP close and a close before the challenge are retried once and the session then carries data exactly; a receiver that always does so sees exactly two connections (never a second retry) |
| `setup_retry_tests::only_failover_takes_a_connect_timeout_as_final` | a connect timeout is retried with `BeforeChallenge` and final with `NotAConnectTimeout`; a refused connect is retried with both |

The four Stage32 F1 tests (`receiver_closing_before_its_challenge_is_retried_once`,
`there_is_never_a_second_retry`, `a_started_challenge_is_never_retried`,
`a_tls_pinning_failure_is_never_retried`) pass unchanged on the direct
path.

## 3. Sandbox (`tools/stage48/retry.py`, 3 clients each)

2026-11-11 20:05–20:11 UTC, this build's lab `werewolfd` `b928e6ff…` (M1
plus the uncommitted change, as the lab's provenance records). Results in
`target/stage48-m2/`.

| Path | `refused` | `blackhole` | `dead` | M1 (`blackhole` / `dead`) |
|---|---|---|---|---|
| encrypted TCP, direct | reset, 0.00 s | reset, 10.00 s | reset, 10.00–10.01 s | same (P3, unchanged by design) |
| failover, B over encrypted TCP, then C | echo via C, 0.01–0.02 s | **echo via C, 5.02–5.04 s** | **echo via C, 5.03–5.04 s** | 10.04 / 10.04–10.06 s |
| failover, B and C over encrypted TCP, then D | echo via D, 0.01–0.02 s | **echo via D, 10.04–10.05 s** | **echo via D, 10.03–10.04 s** | reset at 15.0 s |
| failover, B over QUIC, then C | echo via C, 5.05–5.06 s | 5.05–5.06 s | 5.05 s | same |
| failover, B and C over QUIC, then D | echo via D, 10.05–10.06 s | 10.05 s | 10.04–10.06 s | same |

Every M1 criterion holds: one silent TCP provider 5.0–5.1 s, two reach the
live third in 10.0–10.1 s as QUIC does, the direct Fang unchanged, refused
unchanged. Every daemon stayed up; A's log is empty wherever the clients
echoed.

## 4. Existing checks

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --all-features -D warnings`: PASS.
- `cargo test --workspace --all-features`: **456 passed**, 0 failed (454
  and the two new tests).
- `scripts/ci-local.sh` full (`target/ci-local/20261111T201152Z`): **13
  of 13 PASS**.

## 5. What this does not change

- **Under loss, failover loses one chance per provider.** A provider whose
  SYNs and SYN-ACKs were all lost for the whole 5 s connect window (the
  kernel retransmits within it) is now passed over instead of dialled
  again. F1's validation saw no connect timeout in 600 sessions under 10%
  loss (Stage32 S1-P2); the next provider gets the session instead.
- **P3:** a direct encrypted-TCP Fang to a peer that does not answer still
  resets each client at 10 s; recorded in M4.

## 6. Next

M3 (survey §7): the M1 tool ×3; Stage46 matrix ×1; Stage32 S7 P0
(failover selection), S2 and S4 P0; `ci-local` full.
