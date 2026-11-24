# Stage39 — closure record

**Status: Stage39 CLOSED.** The receiver's connection to its target is now
covered by the session liveness bounds. The fix is reproduced, implemented
and regression-tested in the sandbox.

This record is documentation only. It changes no production source,
harness, dependency, bound or historical verdict.

## 1. Baseline

- `main` `e86a5db` (PR #112 merged).
- tokio 1.52.3, quinn-proto 0.11.18, `Cargo.lock` unchanged by Stage39.
- Tag `v1.0.0-rc.2` is frozen and unchanged. The fix is **not** in the
  rc.2 archive.
- S3-P2 stays EXCLUDED (Stage34). Stage38 stays CLOSED.

## 2. Milestones

| Step | PR | Result | Record |
|---|---|---|---|
| Survey | #109 | Stage39 scoped, D0–D4 approved | [survey](STAGE39_ENGINEERING_PRIORITY_SURVEY.md) |
| M1 reproduction | #110 | CONFIRMED on `main` | [M1](STAGE39_M1_LIVENESS_REPRODUCTION.md) |
| M2 fix | #111 | M2-PASS (`3f1477a`) | [M2](STAGE39_M2_TARGET_LIVENESS_FIX.md) |
| M3 regression | #112 | PASS | [M3](STAGE39_M3_REGRESSION.md) |
| M4 limitations | in #112 | `KNOWN_LIMITATIONS.md` updated | — |

## 3. Owner decisions at closure

- **S39-D5 (real-WAN check of the relay-to-destination path): not run.**
  The owner chose to close without it. The result stays sandbox-only, and
  `KNOWN_LIMITATIONS.md` already says so ("has not been run over a real
  WAN").
- **2026-11-24 follow-up:** the owner authorized a later WAN-check attempt.
  SSH to the previously used Debian VPS (`51.83.41.28:22`) timed out from
  both the sandbox and the host-view retry. No remote command or fault
  injection ran, so S39-D5 remains unrun and no WAN evidence is claimed.
- **Next stage:** hosted CI coverage (survey §13, recommendation 3) is
  complete: specification in PR #184 and implementation in PR #185 are
  merged. The next candidate is QUIC initiator resource-bound parity
  (survey §13.2), which needs its own design and value decisions.

## 4. What remains open (unchanged by this closure)

- The relay-to-destination path over a real WAN (S39-D5, authorized but not
  run because the previously used VPS was unreachable).
- QUIC initiator resource-bound parity (survey §13.2).
- The routed-QUIC regression root cause (Stage34; survey §13.4).
- IPv6 (survey §13.5).

**Not authorised:** tokio promotion, reopening Stage38, rc.3, and release
certification. S39-D5 has since been authorized but remains unrun; see the
follow-up above.
