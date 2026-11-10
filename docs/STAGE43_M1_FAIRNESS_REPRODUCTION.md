# Stage43 M1 — Fairness within a Fang, reproduced

**M1: CONFIRMED.** Both scenarios the survey chose reproduce on `main`,
for all three Fang kinds tested:

- **A, idle connections:** 64 connections that echo once and then sit
  idle still hold all 64 of the Fang's slots after 65 s, and a
  legitimate client is reset.
- **D, one source address:** on a Fang bound to a non-loopback address,
  64 active connections from one source address lock out a client from a
  second address, at once and after 30 s.

No production source changed. One extra finding (§5): when the target
closes first, QUIC and plain-TCP Fangs keep the half-closed session, and
its slot, until the client closes.

## 1. Baseline

- `main` `6ea94e2` plus the survey branch (docs only); lab debug
  `werewolfd` `b1a18eb0…223a`, the Stage42 M2/M3 binary.
- Owner decisions S43-D0–D5 as recommended: F3 (T = 60 s) + F1 (16 of 64
  per non-loopback source), activity measured in the copy paths, F5
  recorded only, a startup warning for non-loopback binds.

## 2. Method

`tools/stage43/fairness.py`, on the Stage42 flood lab (fresh Dens,
dynamic ports, the 50 MB log guard, no live wolf touched). Receiver A
allows only the echo target; B has one Fang of the scenario's kind.
Routed and failover Fangs were not run: they use the same acceptor and
copy paths (Stage42 M2).

- **`idle:<kind>`:** connections are opened one at a time, each echoes
  16 bytes and then stays idle, until the Fang refuses three. A
  legitimate echo is tried at the ceiling, then again after 65 s (past
  the proposed T = 60 s). The echo target has **no idle timeout**; the
  lab's own echo target closes idle connections after 10 s (§5).
- **`source:<kind>`:** in a user, mount and network namespace with a
  private root (as the Stage32 chaos sandbox does; the daemon refuses a
  home whose ancestors belong to an unmapped owner) and a private 512 MB
  `/tmp`. `lo` carries 10.43.0.1–3; the Fang binds 10.43.0.1. Source
  10.43.0.2 opens connections until refused, then sends 16 bytes on each
  every second; 10.43.0.3 tries a legitimate echo before the flood, at
  once, and after 30 s of active flood.

Results: `target/stage43-m1/m1-idle.json`, `m1-source.json`, and
`m1-idle-lab-echo-10s-timeout.json` (§5).

## 3. Scenario A — idle connections (no target timeout)

| Fang | Held | Legit at ceiling | Still held after 65 s idle | Legit after 65 s | B descriptors | Legit after release |
|---|---|---|---|---|---|---|
| QUIC | 64 | **reset** | 64 | **reset** | 82 | PASS |
| encrypted TCP | 64 | **reset** | 64 | **reset** | 145 | PASS |
| plain TCP | 64 | **reset** | 64 | **reset** | 145 | PASS |

Idle connections keep their slots indefinitely: nothing ends a session
for being idle (survey §2). This is the case F3 targets.

## 4. Scenario D — one source address, active flood

| Fang | Legit (10.43.0.3) before | Flood (10.43.0.2) held | Legit at once | Flood open after 30 s active | Legit after 30 s | Legit after release |
|---|---|---|---|---|---|---|
| QUIC | PASS | 64 | **reset** | 64 | **reset** | PASS |
| encrypted TCP | PASS | 64 | **reset** | 64 | **reset** | PASS |
| plain TCP | PASS | 64 | **reset** | 64 | **reset** | PASS |

The flood is active, so F3 alone would not admit 10.43.0.3. This is the
case F1 targets. Admission today ignores the source address
(`fang_accept.rs`), so the result is the same as from loopback; the
namespace is needed so that M2 and M3 can test F1.

In every run of §3 and §4 the daemon stayed up, and the log guard never
triggered. The host `/tmp` stayed at 5%.

## 5. Extra finding: half-closed sessions keep their slot

The first idle run used the lab's echo target, which closes a connection
after 10 s idle (`tests/integration/targets.py`, `settimeout(10)`).
After 65 s every client saw end-of-stream, but:

| Fang | Slots after the target closed | B descriptors | Legit after 65 s |
|---|---|---|---|
| QUIC | still held | 82 | reset |
| encrypted TCP | **freed** | 19 | PASS |
| plain TCP | still held | 145 | reset |

QUIC (`proxy_streams`) and plain TCP (`copy_bidirectional`) pass the
target's FIN to the client and keep the other direction open until the
client closes: ordinary TCP half-close. The encrypted-TCP Fang ends the
whole session when the target ends. Both are existing behaviour, not a
regression. The consequence for Stage43: a client that never closes
after the server has finished holds a slot. Such a session carries no
data, so F3 counts it as idle and can evict it. M2 does not change
half-close semantics.

## 6. Conclusion

Scenarios A and D are CONFIRMED for QUIC, encrypted-TCP and plain-TCP
Fangs. M2 proceeds with the approved design: F3 (evict the longest-idle
connection, idle ≥ 60 s, only when the Fang is full) with activity
recorded in the copy paths, F1 (16 of 64 per non-loopback source
address), and the startup warning for non-loopback binds.
