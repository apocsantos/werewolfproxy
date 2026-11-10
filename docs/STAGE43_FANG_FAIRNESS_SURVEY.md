# Stage43 — Fairness within a Fang (survey)

**Status: survey, for owner review.** Docs only. No production source,
harness, dependency or verdict changes.

Stage42 bounded every Fang: 64 local connections per Fang, 1024 across
the daemon, and anything over a ceiling is reset. That protects the
daemon. It does not protect a Fang's users from each other: whoever fills
a Fang's 64 slots first keeps them, and every later connection is reset
until a slot frees (Stage42 closure §3). This survey asks which clients
can be told apart, and what a fairer admission rule would cost.

## 1. Baseline

- `main` `6ea94e2` (PR #123 merged, Stage42 CLOSED).
- tokio 1.52.3, quinn-proto 0.11.18. `libc` is in `Cargo.lock` (through
  `socket2`, tokio) but is not a direct dependency of `werewolfd`;
  nothing in `werewolfd` reads `/proc`.
- Tag `v1.0.0-rc.2` frozen. S3-P2 and S2-P4 stay EXCLUDED.

## 2. What admission looks like today (CONFIRMED from code)

`transport/fang_accept.rs`, `FangAcceptor::accept`:

1. `listener.accept()`; on error back off 100 ms.
2. `try_acquire_owned` a per-Fang permit (64), then a global one (1024).
3. Either fails → the new connection is reset at once.

The slot (`FangSlot`) is held by the connection's task until the session
ends. Nothing in the acceptor knows who the client is, when it last sent
or received data, or how to end an existing session.

How long a session lives:

- No Fang kind has an application idle timeout. An idle session lives as
  long as the path does: QUIC sends keep-alives every 10 s
  (`quic_fang.rs:16`), encrypted TCP relies on kernel keep-alive
  (`tcp_encrypted.rs:25`, `:596`), plain TCP has neither.
- So a connection that is opened and never used holds its slot
  indefinitely. That is what the Stage42 flood did, and also what a
  leaking client (one that opens connections and forgets them) does.

## 3. Who can be told apart

| Client | Source address seen by the Fang | Distinguishable by |
|---|---|---|
| Local process, loopback bind (the common case) | `127.0.0.1` for every client | nothing in the address; on Linux the client socket's **uid** in `/proc/net/tcp` |
| Two local users on one host | `127.0.0.1` | uid only |
| Host on the LAN, non-loopback bind | its own IP | **source IP** |
| Many processes of one user | `127.0.0.1` | nothing practical: same trust domain |

**Consequence:** a per-source-address limit, the usual answer, does
nothing for a loopback Fang. Every local client shares one address. It
only helps Fangs bound to a non-loopback address (S42-D5).

**What no option here can do:** separate an active flood from a
legitimate client of the same user on the same host. Both are the same
principal sending real traffic. Stage43 should not claim to.

## 4. Scenarios

| # | Scenario | Today |
|---|---|---|
| A | A buggy local client leaks idle connections and fills the Fang | every later client reset until the leaker exits |
| B | An active flood from the operator's own user | every later client reset while it lasts |
| C | Another local user on a multi-user host floods a loopback Fang | same as B |
| D | One LAN host floods a non-loopback Fang | every LAN client reset |
| E | Two Fangs to the same peer compete for the receiver's 64-session quota (`authority.rs:18`) | first come, first served across the Fangs |

A and D are the realistic ones on a private deployment: a leaky client
and a LAN-shared Fang. B is out of reach (§3). C needs a multi-user host.
E is a different problem (the remote quota, not local admission).

## 5. Options

### F1. Per-source-IP share (non-loopback sources only)

At most N of a Fang's 64 slots per source IP (proposal: 16). Loopback
sources are exempt; otherwise one local address would get only 16 of 64.

- Helps D. Nothing else.
- Cost: a small map (IP → count) in the acceptor, released with the
  slot. No data-path change.
- Test: needs a non-loopback source. On Linux all of `127/8` is loopback,
  so the lab cannot use `127.0.0.2`. Use the Stage32 unshare sandbox
  (add an address in its network namespace) or a unit test with an
  injected exemption rule.

### F2. Per-local-user share (Linux, loopback)

For a loopback connection, find the client socket's uid in
`/proc/net/tcp` (`/proc/net/tcp6` for IPv6), and give each uid at most N
slots.

- Helps C. Not A or B (one user).
- Cost: a scan of `/proc/net/tcp` per accept, O(sockets on the host).
  Under a flood that is the expensive path, and it runs at the worst
  moment. A race: the client can close before the lookup. Linux only.
  Parsing `/proc` is new code with its own failure modes.
- Value on this project's hosts (single operator) is low.

### F3. Evict the longest-idle connection under pressure

When a Fang is at its ceiling and a new connection arrives: if its
longest-idle connection has been idle (no bytes either way) for at least
T (proposal: 60 s), end that session (RST to its client) and admit the
newcomer. Otherwise reset the newcomer, as today. The global ceiling
stays reset-only.

- Helps A, and B/C/D whenever the flood's connections are idle (the
  Stage42 flood's were). An active flood still wins.
- No change at all while the Fang is under its ceiling: idle sessions
  are only at risk when the Fang is full.
- Cost:
  - each slot gets a last-activity timestamp and a way to end its
    session (a cancel signal the connection's task selects on);
  - the forwarders must update the timestamp as bytes pass. The five
    accept loops keep their concrete `TcpStream` (reset arming needs it),
    so the touch goes into the shared copy paths:
    `copy_bidirectional` (plain), `proxy_streams` (QUIC, also used by
    failover), `secure_copy_client_side` (encrypted TCP, routed,
    failover). That is a **data-path change**: one relaxed atomic store
    per chunk.
  - Alternative without touching the data path: the kernel's
    `TCP_INFO` (`tcpi_last_data_recv`/`_sent`) on the local socket.
    `socket2` 0.6 does not expose it, so it needs `libc` as a direct
    dependency and `unsafe` `getsockopt`. A dependency decision.
- Cost to users: under pressure, a legitimate session idle for ≥ T
  (an idle SSH session, say) can be ended to admit a newcomer. That is
  the trade, and it must be stated.

### F4. Operator-set ceiling per Fang

A `max_connections` field on the Fang/profile so the operator can size
each Fang. Changes the state format (`werewolf-core` `fang.rs`,
`fang_profile.rs`) and the control API. Does not add fairness by itself.

### F5. Per-Fang share of a peer's remote quota (scenario E)

Partition the receiver's 64 sessions per peer among the initiator's
Fangs to that peer. The initiator does not know the receiver's quota
except by convention, and Fangs come and go. Needs its own design;
recorded, not proposed.

## 6. Recommendation

**F3 with the timestamp in the copy paths, plus F1.** Defer F2, F4, F5.

- F3 covers the realistic case (A) and every idle flood, on every bind,
  with no new dependency and no state-format change. It never affects a
  Fang that is under its ceiling.
- F1 is cheap and covers the LAN case (D) that F3 misses when the LAN
  host's connections are active.
- F2's cost (a `/proc` scan per accept, under flood) is too high for a
  case this project's hosts do not have.
- B stays unsolved, and the records say so.

**S42-D5 (bind address), carried:** recommend a single startup log line
when a Fang binds a non-loopback address ("reachable from the network").
No behaviour change, no state change.

## 7. Proposed Stage43 contract (for owner review)

- **M1, reproduction first** (lab, disposable, no live wolves; the 50 MB
  log guard stays):
  - scenario A: a client opens 64 idle connections to a Fang of each
    kind, waits past T, then a legitimate client connects. Today: reset.
  - scenario D: in the unshare sandbox, a Fang bound to a non-loopback
    address; one source floods with active traffic, a second source
    connects. Today: reset.
  - M1 stops the stage if neither reproduces.
- **M2, fix:** F3 and F1 in `FangAcceptor` plus the activity touch in the
  five copy paths; the S42-D5 log line. Unit tests: idle eviction picks
  the longest-idle; a session idle < T is never evicted; an evicted
  session's client sees a reset and its slot is reused; the per-IP share
  applies to non-loopback only; no slot leaks. Workspace tests, fmt,
  clippy, `scripts/ci-local.sh` full.
- **M3, regression:** the M1 and Stage42 flood scenarios ×3; because the
  data path changes, also the Stage32 throughput cells the Stage41 M3
  ran (S2 P0/1/2/5, S3 P0/1, S4 P0/2, S5 P0, S7 P0) and the routed-Fang
  gate. Every historical verdict must hold.
- **M4:** `KNOWN_LIMITATIONS` update and closure.

## 8. Risks

- **Ending legitimate idle sessions.** Only when the Fang is full, only
  sessions idle ≥ T, and only one per admitted newcomer. Logged at the
  Stage42 bounded rate.
- **Data-path cost.** One atomic store per chunk. M3 runs the throughput
  cells to show no regression.
- **Churn under a reconnecting flood.** An evicted flooder reconnects and
  may evict an idle legitimate session in turn. Fairer than today, not
  fair. Stated.
- **Clock.** Idle time uses the monotonic clock (`tokio::time::Instant`).

## 9. Owner decisions

| ID | Decision | Options | Recommended |
|---|---|---|---|
| S43-D0 | Approve Stage43 as scoped (§7) | approve, amend, reject | approve |
| S43-D1 | Fairness mechanisms | F1, F2, F3, F4 (any combination), none | F3 + F1 |
| S43-D2 | Values | idle threshold T = 60 s; per-source share 16 of 64; other | 60 s / 16 |
| S43-D3 | How F3 measures activity | touch in the copy paths (no new dependency); `TCP_INFO` (adds `libc`, `unsafe`) | copy paths |
| S43-D4 | Scenario E (Fangs sharing a peer's quota) | separate stage; record only | record only |
| S43-D5 | Bind address (S42-D5 carried) | startup warning line; document only; loopback-only default | warning line |
