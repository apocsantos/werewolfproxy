# Stage45 M1 — Fang failure visibility, reproduced

**M1: P1–P5 CONFIRMED.**

- **P1:** when a session fails to set up, QUIC, encrypted-TCP, routed and
  failover Fangs log nothing. This holds whether the receiver withholds its
  grant or the peer's address is dead.
- **P2:** a plain-TCP Fang whose target is down logs one line per
  connection. 500 connections produced 501 lines (52.6 kB) in 0.1 s.
- **P3:** `fang.list` is unchanged after every failure: `Active`, with
  the same fields.
- **P4:** discovery reports a local QUIC dial failure as `rejected`.
- **P5:** a peer whose Pack address is `localhost:PORT` is accepted by
  `pack add`. Its QUIC Fang opens, then resets every session and logs
  `bad server addr` once per connection, with no Fang id. The same peer
  over TCP echoes every time.

No production source changed.

## 1. Baseline

- `main` `d409048` plus the survey branch (docs only). Lab debug
  `werewolfd`
  `a5d259d32695dd919f05b9edfd0a25d4de273296b0ea435648f4a3082dea4a96`,
  the Stage44 M2/M3 binary (no source change since).
- Owner decisions S45-D0–D5 as recommended:
  - L1: a rate-limited setup-failure log per Fang, classed `dial`,
    `handshake` or `target`;
  - plain TCP moves onto the limiter;
  - S1a: a local dial error reports `unreachable`;
  - C1: `sessions_failed` and `last_failure` in `fang list`;
  - H1: a QUIC Fang resolves its peer address once at open.

## 2. Method

`tools/stage45/visibility.py`, run 2026-11-10 18:12–18:16 UTC, once per
scenario, each in a fresh lab on IPv4 loopback with the 50 MB log guard.

- A is the initiator, whose log and `fang.list` are measured. B and C are
  receivers.
- Each scenario opens one Fang on A with `service connect`. It then runs
  10 clients one at a time; each sends 16 bytes and waits for the echo.
- It records each client's outcome, every line A logged during the
  clients, and A's `fang.list` entry before and after.

The causes:

- **`grant`:** everything is reachable, but the final receiver does not
  grant the target. That is a refusal during the handshake.
- **`dial`:** the first hop's address is a reserved, non-listening port.
  For each kind that means:
  - QUIC and TCP: B's address;
  - routed: the relay B;
  - failover: both providers;
  - plain TCP: the target itself.

Results: `target/stage45-m1/m1-visibility.json` and
`m1-visibility.log`.

## 3. Results

| Scenario | Clients | Time | Lines A logged | A's `fang.list` after |
|---|---|---|---|---|
| `grant:quic` | reset ×10 | 0.2 s | **0** | `Active`, `quic` |
| `grant:tcp` (encrypted) | reset ×10 | 0.1 s | **0** | `Active`, `tcp` |
| `grant:routed` (QUIC hop) | reset ×10 | 0.4 s | **0** | `Active`, `tcp via wolf-b (quic)` |
| `grant:failover` (B, C) | reset ×10 | 0.4 s | **0** | `Active`, `providers: wolf-b, wolf-c` |
| `dial:quic` | reset ×10 | 50 s (5 s each) | **0** | `Active` |
| `dial:tcp` | reset ×10 | 0.0 s | **0** | `Active` |
| `dial:tcp-plain` | reset ×10 | 0.0 s | **11** (one per connection, plus the lab's readiness probe) | `Active` |
| `dial:routed` | reset ×10 | 50 s | **0** | `Active` |
| `dial:failover` | reset ×10 | 100 s (both providers) | **0** | `Active` |
| `flood:tcp-plain`, 500 connections | reset ×500 | 0.1 s | **501**, 52,605 bytes | `Active` |
| `hostname:quic` | reset ×10 | 0.0 s | **10** × `bad server addr: invalid socket address syntax` | `Active`; `pack add` exit 0 |
| `hostname:tcp` | **echo ×10** | 0.1 s | 0 | `Active` |

In every scenario the daemons stayed up and the log guard never
triggered.

**P4, unit test.** `discovery::tests::a_local_quic_dial_failure_is_unreachable`
asks `query_quic` to dial `127.0.0.1:0`. Quinn refuses that destination
locally, before any packet is sent, as it refused an IPv6 destination
before Stage44. `failure_status` then returns `rejected`, where the
contract says `unreachable`. The test is `#[ignore]` until M2; with
`--include-ignored` it fails with `left: "rejected"`, `right:
"unreachable"`.

## 4. Notes

- **Not run in the lab:** P4 end to end. Since Stage44 no production
  address triggers a local QUIC dial error. M1 therefore uses the unit
  test, as the survey proposed, rather than adding a production switch.
- **Plain-TCP growth** is about 105 bytes per failed connection, at
  whatever rate clients retry. Admission bounds concurrency (64), not
  rate.
- **The `bad server addr` line** carries no Fang id, so even when logged
  it does not say which Fang is failing.
- **A dead QUIC peer costs 5 s per session** (`READ_WINDOW`) before the
  reset, so a client sees a slow failure as well as a silent one.
  Stage45 does not change that window.

## 5. Conclusion

P1–P5 are CONFIRMED. M2 proceeds with L1, S1a, C1 and H1 as approved.
