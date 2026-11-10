# Stage42 — closure record

**Status: Stage42 CLOSED.** Every Fang kind now has a connection ceiling
and survives descriptor exhaustion. The flood exposure was reproduced,
fixed and regression-tested.

This record is documentation only: no production source, harness,
dependency, bound or historical verdict changes.

## 1. Baseline

- `main` after PR #122 (M3) merged.
- tokio 1.52.3, quinn-proto 0.11.18, `Cargo.lock` unchanged by Stage42.
- Tag `v1.0.0-rc.2` is frozen and unchanged. The fix is **not** in the
  rc.2 archive.
- S3-P2 and S2-P4 stay EXCLUDED. Stage38 stays CLOSED.

## 2. Milestones

| Step | PR | Result | Record |
|---|---|---|---|
| Survey | #119 | ceilings A + B, 64 / 1024, reset over the ceiling, accept back-off; S42-D0–D5 as recommended; Stage41 limitation corrected | [survey](STAGE42_FANG_CONNECTION_CEILING_SURVEY.md) |
| M1 test | #120 | CONFIRMED: self-DoS at the 64-session quota; encrypted/plain-TCP Fangs die on `EMFILE`; QUIC Fang spins (103% CPU); plain-TCP unbounded (1017 fds). `/tmp` incident recorded | [M1](STAGE42_M1_FANG_FLOOD_REPRODUCTION.md) |
| M2 fix | #121 | M2-PASS (`e9fb6e2`); `FangAcceptor` in all five Fang kinds; 418 tests; local CI 13/13 | [M2](STAGE42_M2_FANG_CEILING_FIX.md) |
| M3 regression | #122 | PASS: flood scenarios ×3, S4-P0/P2 3/3, routed-Fang gate 12/12; the 173-descriptor item is not a leak | [M3](STAGE42_M3_REGRESSION.md) |
| M4 | this PR | `KNOWN_LIMITATIONS.md` updated; closure | — |

## 3. What remains open

- **No per-client fairness within a Fang.** A flood on one Fang still
  denies that Fang's other users while it lasts, and Fangs to the same
  peer share its 64-session quota. Fixing that needs per-client
  accounting: a later stage with its own survey, if wanted.
- **Bind address** (S42-D5): a Fang bound to a non-loopback address
  accepts from that whole network. Documented only; a loopback-only
  default would change documented behaviour and needs its own decision.
- **Tooling:** `tools/stage42/fang_flood.py` must keep its 50 MB log
  guard (M1 §5).
- From earlier stages: the initiator's stream receive and send windows at
  Quinn's defaults (Stage41; the send window is the Stage34 surface);
  IPv6; the routed-QUIC regression root cause (Stage34); the real-WAN
  relay-to-destination check (S39-D5).

**Not authorised:** tokio promotion, rc.3, release certification, WAN
testing.
