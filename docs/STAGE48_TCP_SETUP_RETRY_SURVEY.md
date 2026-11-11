# Stage48 — the encrypted-TCP setup retry on a dead peer (survey)

**Status: survey, for owner review.** Docs only. No production source,
harness, dependency or verdict changes. The probe below used a scratch
copy of `tools/stage46/dial.py` (two more address shapes and a failover
whose first provider is reached over TCP), since removed; results in
`target/stage48-survey/`.

Stage46 and Stage47 left open that an encrypted-TCP Fang whose peer does
not answer takes 10 s per client, because the Stage32 F1 retry repeats
the whole dial. This survey measures it on every path that uses the
retry. **It matters most for failover:** a silent encrypted-TCP provider
holds each local connection 10 s before the next provider is tried,
twice what a silent QUIC provider costs, and two of them use up the
15 s selection window, so a third provider is never reached.

## 1. Baseline

- `main` `2829801` (PR #149 merged, Stage47 CLOSED). Lab debug
  `werewolfd` `d226a528…`.
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18, rustix 1.1.6.
- Tag `v1.0.0-rc.2` frozen. S3-P2 and S2-P4 stay EXCLUDED.

## 2. The retry (CONFIRMED from code and its record)

`connect_tcp_session` (`transport/tcp_encrypted.rs`; Stage32 F1, owner
decision 2026-10-27, [record](STAGE32_F1_SETUP_RETRY.md)) makes one more,
fully independent attempt when the first fails before any byte of the
receiver's challenge. It is used by the direct encrypted-TCP Fang and the
encrypted-TCP leg of failover; routed hops and discovery do not use it.

Retried: a TCP connect failure (refused, unreachable, **5 s timeout**), a
TLS transport failure (reset, close, 5 s timeout), a close or a 5 s
challenge read with no challenge byte. Not retried: a TLS alert or pin
failure, a started challenge, the retry itself.

The defect F1 fixed was loss **after** TCP connected (Stage32 S1-P2: a
lost TLS ChangeCipherSpec, then the receiver's 5 s window closing before
the challenge). In F1's validation (600 sessions under P2, before the
fix) no session failed on a TCP connect timeout: the 5 s connect window
already carries the kernel's SYN retransmissions. The connect-timeout
retry was included because it fell inside the "before the challenge"
boundary, not because a case needed it.

## 3. Probe (2026-11-11, Stage46 sandbox, 3 clients each)

Shapes: `refused` (`127.0.0.2`, nothing listening), `blackhole`
(`10.46.0.9`, dropped silently), `dead` (both), as Stage46's names.

| Path | `refused` | `blackhole` | `dead` |
|---|---|---|---|
| encrypted TCP | reset, 0.00 s | **reset, 10.0 s**; `dial`, "tcp connect timeout" ×2 per client | **reset, 10.0 s** |
| plain TCP (target) | — | reset, 5.0 s; `target` | (Stage46: 5.0 s) |
| QUIC | — | reset, 5.0 s; `dial` | (Stage46: 5.0 s) |
| failover, B over QUIC then C | — | — | echo via C, 5.05 s |
| failover, **B over encrypted TCP** then C | echo via C, 0.01 s | **echo via C, 10.05 s** | **echo via C, 10.04 s** |

`failover-tcp:fast` echoes via B in 0.01 s (the probe's control).

## 4. Problems

- **P1, failover delayed (CONFIRMED).** A silent encrypted-TCP provider
  costs each local connection 10 s before the next provider, against 5 s
  for QUIC. Failover is itself the retry: the next provider is the
  alternative the connect-timeout retry tries to provide.
- **P2, a third provider unreached (code reading, not measured).**
  `select_and_serve` gives all providers together `hs::CLIENT_WINDOW`
  (15 s). Two silent encrypted-TCP providers use 10 s + the remaining 5 s,
  and the window ends during the second: the connection is reset and
  "selection window ended" is logged, though a third provider would
  answer. With QUIC providers the third is reached at about 10 s.
- **P3, direct Fang 10 s on a dead peer (CONFIRMED).** Each local client
  waits 10 s for a reset instead of 5 s (QUIC, plain TCP). No other
  provider exists, so the retry costs only time; whether it ever rescues
  a session is the question in §2.

A refused or unreachable peer fails in milliseconds and its retry costs
nothing; only the timeout matters.

## 5. Options

- **O1, failover only:** in the encrypted-TCP leg of failover, a TCP
  connect timeout is final (the next provider is tried at once). Every
  other F1 case is still retried there, and the direct Fang is unchanged.
  Silent TCP provider: 10 s → 5 s, as QUIC; P2 goes with it.
- **O2, everywhere:** a TCP connect timeout is never retried, on the
  direct Fang too (P3: 10 s → 5 s). Changes the F1 boundary the owner set
  for the direct path, so it needs the F1 loss validation rerun
  (Stage32 S1-P2, 600 sessions): a connect lost for the whole 5 s would no
  longer get a second chance.
- **O3, a shared budget:** keep the retry but bound both attempts' connect
  by one 5 s window. Same effect as O2 on a dead peer, more code, same
  validation need.
- **O0, accept:** record P1–P3 with the measured figures.

## 6. Recommendation

**O1.** It removes the worst case (P1, P2), where the retry delays a
real alternative, and leaves the direct path, where the retry is F1's
owner-set behaviour and its only cost is time, unchanged and recorded.
O2 is the follow-up if the owner wants the direct path at 5 s; it should
come with the S1-P2 loss rerun.

## 7. Proposed Stage48 contract (for owner review)

- **M1, reproduction:** a tool (`tools/stage48/`, built on the Stage46
  sandbox) for: encrypted TCP `refused` / `blackhole` / `dead`; failover
  with B over encrypted TCP on those shapes; failover with **three**
  providers, B and C silent over encrypted TCP and D live (confirms P2).
  Same on QUIC providers as the comparison.
- **M2, fix:** O1 in `transport/failover.rs` / `tcp_encrypted.rs` (a
  connect-timeout-final variant of `connect_tcp_session` for the failover
  leg); unit tests that failover does not retry a connect timeout and
  still retries a TLS transport failure and a close before the challenge,
  and that the direct path still retries all three. Criterion: silent TCP
  provider 5.0–5.1 s; three-provider case echoes via D.
- **M3, regression:** the M1 tool ×3; Stage46 matrix ×1; F1 tests
  unchanged; Stage32 S7 P0 (failover selection) and S2, S4 P0; `ci-local` full.
- **M4:** `KNOWN_LIMITATIONS` and closure, including P3 if O2 is not
  chosen.

No wire, trust, Target Policy or receiver change: the receiver never sees
the attempt that is no longer made.

## 8. Owner decisions

| ID | Decision | Options | Recommended |
|---|---|---|---|
| S48-D0 | Approve Stage48 as scoped (§7) | approve, amend, reject | approve |
| S48-D1 | Connect timeout in the setup retry | O1 failover only; O2 everywhere (with S1-P2 rerun); O3 shared budget; O0 accept | O1 |
| S48-D2 | Direct Fang, dead peer (P3) | record as a limitation; include O2 in this stage | record |
