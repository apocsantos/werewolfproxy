# Stage42 M3 — Regression

**M3: PASS.** The M1 flood scenarios hold on the M2 build in all three
rounds. Stage32 S4-P0 and S4-P2 pass 3 of 3 seeds each. The routed-Fang
gate passes. Every historical verdict holds. The M1 "UNEXPLAINED" item
(173 descriptors after the plain-TCP flood) is not a leak on this build.

## 1. Build

- Tested commit `2f2c1f5` (M2 branch head). Its tree is identical to
  `main` `d7bffd2` (the #121 merge); `git diff 2f2c1f5 d7bffd2` is empty.
- Lab debug `werewolfd`
  `b1a18eb0072b422069e7dfef098820fa4c38f8111d512e9cc5ebb04b0521223a`,
  the same binary as the M2 check.
- Run 2026-11-09 22:04–22:28 UTC. `/tmp` stayed at 5% throughout.
- One tooling change, in this PR: `fang_flood.py`'s quota scenario also
  reads B's descriptors 30 s after release (`fds_released_30s`), to tell
  slow teardown from a leak. No production source changed.

Results: `target/stage42-m3/` (`r{1,2,3}-{256,64}.json`, logs,
`meta.txt`), chaos `target/stage32-chaos/results/20261109T221811Z-network`,
routed gate `target/stage42-m3/routed.log`.

## 2. Descriptor exhaustion (`fd:<kind>`), three rounds

| Scenario | B descriptors flooded (r1/r2/r3) | CPU flooded | After release (all rounds) |
|---|---|---|---|
| fd:quic, `nofile` 256 | 73 / 76 / 82 | 0% | 18 descriptors, listening, listed, legit echo PASS |
| fd:tcp, 256 | 137 / 145 / 145 | 0% | 17, listening, listed, legit PASS |
| fd:tcp-plain, 256 | 145 / 145 / 145 | 0% | 17, listening, listed, legit PASS |
| fd:quic, **64** | 64 / 64 / 64 | 1.6% / 1.6% / 1.6% | 18, listening, listed, legit PASS |
| fd:tcp, **64** | 63 / 63 / 63 | 1.6% / 1.4% / 1.8% | 17, listening, listed, legit PASS |

On `main` before M2 (M1): the encrypted-TCP and plain-TCP Fangs stopped
listening for good, and the QUIC Fang at 64 spun at 103% CPU. In M3 the
log guard never triggered and the daemon stayed up in all 15 runs.

## 3. Session quota (`quota:<kind>`), 500 connections, three rounds

| Fang | Echoed (r1/r2/r3) | B descriptors flooded | Released at 5 s / 30 s | Legit during (r1/r2/r3) | Legit after |
|---|---|---|---|---|---|
| QUIC | 61 / 59 / 59 | 79 / 77 / 77 | 18 / 18 | PASS / PASS / PASS | PASS ×3 |
| encrypted TCP | 55 / 60 / 64 | 127 / 137 / 145 | 17 / 17 | PASS / PASS / FAIL | PASS ×3 |
| plain TCP | 64 / 64 / 64 | 145 ×3 | 17 / 17 | FAIL ×3 | PASS ×3 |

- **Plain TCP is bounded.** 64 sessions and 145 descriptors in every
  round; on `main` it was 422 sessions and 1017 descriptors.
- **"Legit during" is the stated M2 limit, not a regression.** When the
  flood holds all 64 of the Fang's slots, a new connection is reset. It
  passes only when some flood sessions failed admission at the receiver
  (QUIC handshakes, 16 concurrent per peer) and freed slots: incidental,
  as the M2 record says.
- **The 173-descriptor item: resolved as not a leak on this build.** Every
  quota run is at 17–18 descriptors 5 s after release and unchanged at
  30 s, the same as idle. With the ceiling the plain-TCP flood never has
  more than 64 sessions to tear down. Whether `main` before M2 leaked or
  was slow to tear down 422 sessions was not re-measured (that build is
  superseded); either way it does not occur now.

## 4. Stage32 S4 (`chaos.py network --cells S4:P0,S4:P2`)

| Cell | Seeds | Verdict | Historical |
|---|---|---|---|
| S4-P0 | PASS, PASS, PASS | PASS | PASS |
| S4-P2 | PASS, PASS, PASS | PASS | PASS |

## 5. Routed-Fang gate (`tests/integration/routed_fang_gate.py`)

PASS, 12 of 12 tests, including 64 MiB full duplex over a QUIC hop and
over an encrypted-TCP hop, Silver on the relay cutting a live routed
session, a relay address change closing the routed Fang, and plain TCP
refused for a routed peer.

## 6. Conclusion

All M3 criteria hold. M4: `KNOWN_LIMITATIONS` update and Stage42
closure, carrying the stated limit (no per-client fairness within a
Fang; Fangs to one peer share its 64-session quota).
