# Stage43 — closure record

**Status: Stage43 CLOSED.** Idle connections no longer hold a full Fang
indefinitely, and one source address can no longer lock other hosts out
of a Fang bound to a non-loopback address. Both exposures were
reproduced, fixed and regression-tested.

This record is documentation only: no production source, harness,
dependency, bound or historical verdict changes.

## 1. Baseline

- `main` after PR #127 (M3) merged.
- tokio 1.52.3, quinn-proto 0.11.18, `Cargo.lock` unchanged by Stage43
  (no new dependency, S43-D3).
- Tag `v1.0.0-rc.2` is frozen and unchanged. The fix is **not** in the
  rc.2 archive.
- S3-P2 and S2-P4 stay EXCLUDED. Stage38 stays CLOSED.

## 2. Milestones

| Step | PR | Result | Record |
|---|---|---|---|
| Survey | #124 | F3 idle eviction (T = 60 s) + F1 per-source share (16 of 64, non-loopback); activity in the copy paths; F2, F4, F5 deferred; startup line for non-loopback binds; S43-D0–D5 as recommended | [survey](STAGE43_FANG_FAIRNESS_SURVEY.md) |
| M1 test | #125 | CONFIRMED for QUIC, encrypted TCP and plain TCP: 64 idle connections hold a Fang after 65 s; one source address locks out another. Half-close finding recorded | [M1](STAGE43_M1_FAIRNESS_REPRODUCTION.md) |
| M2 fix | #126 | M2-PASS (`faa1da2`): idle eviction, per-source share, `Tracked` activity in the shared copy paths, bind warning; 424 tests; local CI 13/13 | [M2](STAGE43_M2_FAIRNESS_FIX.md) |
| M3 regression | #127 | PASS: M1 scenarios ×3 all kinds; Stage42 flood scenarios ×3; Stage32 cells S2 P0/1/2/5, S3 P0/1, S4 P0/2, S5 P0, S7 P0 3/3 each; routed-Fang gate 12/12 | [M3](STAGE43_M3_REGRESSION.md) |
| M4 | this PR | `KNOWN_LIMITATIONS.md` updated; closure | — |

## 3. What remains open

- **Active loopback flood (survey scenarios B and C).** An active flood
  from the operator's own user, or from another local user, still fills
  a Fang while it lasts. A per-local-user share (F2) was deferred: a
  `/proc/net/tcp` scan per accept, under flood, Linux only, for a case
  this project's hosts do not have.
- **Idle sessions under pressure.** While a Fang is full, a legitimate
  session idle for 60 s or more can be ended to admit a newcomer: the
  approved trade (S43-D1/D2).
- **Shared peer quota (scenario E, F5).** Fangs to one peer share its
  64-session quota, first come, first served. Recorded only (S43-D4);
  needs its own design.
- **Operator-set ceiling per Fang (F4).** Not done; it would change the
  state format and control API.
- **Half-close** (M1 §5): QUIC and plain-TCP Fangs keep a half-closed
  session until the client closes. Existing behaviour, unchanged; such a
  session is idle and can be evicted.
- **Impaired-cell spread** (M3 §4): S2-P5 and S4-P2 passed but spread
  wider than in Stage41; attributed to the injected impairment, not
  investigated.
- **Tooling:** `tools/stage42/fang_flood.py` must keep its 50 MB log
  guard; `tools/stage43/fairness.py` reuses it.
- From earlier stages: the initiator's stream windows at Quinn's
  defaults (Stage41; the send window is the Stage34 surface); IPv6; the
  routed-QUIC regression root cause (Stage34); the real-WAN
  relay-to-destination check (S39-D5).

**Not authorised:** tokio promotion, rc.3, release certification, WAN
testing.
