# Stage41 — closure record

**Status: Stage41 CLOSED.** The QUIC initiator no longer gives a remote
receiver resources it never uses. The exposure was reproduced, fixed and
regression-tested.

This record is documentation only: no production source, harness,
dependency, bound or historical verdict changes.

## 1. Baseline

- `main` `86899a8` (PR #117 merged).
- tokio 1.52.3, quinn-proto 0.11.18, `Cargo.lock` unchanged by Stage41.
- Tag `v1.0.0-rc.2` is frozen and unchanged. The fix is **not** in the
  rc.2 archive.
- S3-P2 and S2-P4 stay EXCLUDED. Stage38 stays CLOSED.

## 2. Milestones

| Step | PR | Result | Record |
|---|---|---|---|
| Survey | #115 | Option A chosen; S41-D0–D5 as recommended | [survey](STAGE41_QUIC_INITIATOR_BOUNDS_SURVEY.md) |
| M1 test | #116 | CONFIRMED: peer streams (uni, bidi) and datagrams; 249 MB accepted unread | [M1](STAGE41_M1_INITIATOR_EXPOSURE.md) |
| M2 fix | #117 | M2-PASS (`5449261`); 1.29 MB; 414 tests; local CI 13/13 | [M2](STAGE41_M2_INITIATOR_BOUNDS_FIX.md) |
| M3 regression | #117 | PASS, 10 cells, 30/30 runs | [M3](STAGE41_M3_REGRESSION.md) |
| M4 | this PR | `KNOWN_LIMITATIONS.md` updated; closure | — |

## 3. What remains open

- **Fang connection ceiling** (S41-D5): a later stage with its own
  survey.
- The initiator's stream receive and send windows stay at Quinn's
  defaults (a throughput decision; the send window is the Stage34
  surface).
- From earlier stages: IPv6; the routed-QUIC regression root cause
  (Stage34; retest on quinn-proto > 0.11.19, which was not released on
  2026-11-09); the real-WAN relay-to-destination check (S39-D5).

**Not authorised:** tokio promotion, rc.3, release certification, WAN
testing.
