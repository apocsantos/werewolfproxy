# Stage45 — Fang failure visibility (survey)

**Status: survey, for owner review.** Docs only. No production source,
harness, dependency or verdict changes.

Stage44 M1 found that a QUIC Fang to an IPv6 peer reset every session
while `fang active` showed it `active` and neither daemon logged a line,
and that discovery and sync reported the same local failure as
`rejected`, the status of a trust refusal. Stage44 removed that cause;
any other cause would be just as invisible. This survey reads what an
operator can see today when a Fang's sessions fail, and proposes the
smallest change that makes it visible.

## 1. Baseline

- `main` `d409048` (PR #133 merged, Stage44 CLOSED).
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18.
- Tag `v1.0.0-rc.2` frozen. S3-P2 and S2-P4 stay EXCLUDED.

## 2. What an operator sees today (CONFIRMED from code)

**Per Fang kind, when a session fails to set up:**

| Fang kind | Where | On a setup failure |
|---|---|---|
| QUIC | `quic_fang.rs` session task: `let Ok(..) = connect_v3_on(..) else { return }` | **nothing logged**; the client sees a reset |
| encrypted TCP | `tcp_encrypted.rs` `pipe_one_fang_connection`; the accept loop keeps only `Some(Ok(()))` | **nothing logged** |
| routed (QUIC or TCP hop) | `relay.rs` `routed_session` | **nothing logged** |
| failover | `failover.rs` (provider skipped only before establishment, Stage27) | **nothing logged**, including when every provider fails |
| plain TCP | `tcp_plain.rs` | one `eprintln!` **per failed connection** (target connect error, 5 s timeout, pipe error), **not rate-limited** |

Other lines exist only for the Fang opening, a bind failure, the
Stage42/43 admission events (rate-limited, one line per 10 s per Fang per
kind, `fang_accept.rs` `RateLimitedLog`) and an unparsable QUIC server
address (`bad server addr`, `eprintln!` per connection, unbounded).

**The QUIC server address is not checked when the Fang opens.** A Pack
address may be a hostname (`state_validation.rs` `endpoint()` requires
only a non-empty host and a port). The open path (`main.rs`, Fang open)
passes the text through; the QUIC Fang parses it as a literal
`SocketAddr` per connection (`quic_fang.rs:89`). So a QUIC Fang to a
peer whose Pack address is a hostname opens successfully, then fails
every session and logs `bad server addr` once per connection. The same
peer works over TCP (`TcpStream::connect` resolves) and for discovery and
sync over QUIC (`lookup_host`, first address). Not yet run.

**Errors are collapsed before anything could report them.** In
`connect_v3_on` every failure becomes `hs::rejected()` (`PermissionDenied`,
"handshake rejected"): a local `endpoint.connect` error (the Stage44 case),
a handshake that never completes (`READ_WINDOW`), a closed connection, a
failed OPEN/ACK or signature check, and the receiver refusing the target.
The encrypted-TCP path keeps the TCP connect error's kind (refused, no
route, timeout) and collapses the rest.

**`fang list` / `fang active`** (`control.rs`): `id`, `peer`, `local`,
`remote`, `state` (always `active` while the listener runs),
`transport`, `uptime_seconds`. No session count, no failure count, no last
error.

**Discovery and sync statuses** (`discovery/operator.rs`
`failure_status`): `unreachable` for `ConnectionRefused`,
`HostUnreachable`, `NetworkUnreachable`, `AddrNotAvailable`, `TimedOut`;
everything else `rejected`. `docs/V1_CLI_CONTRACT.md` defines
`unreachable` as "no connection was made (refused, no route, or no answer
to the dial)" and `rejected` as "the node was reached and the handshake,
authentication or result check failed". `rpc_quic` (`discovery.rs`) maps a
local `endpoint.connect` error to `hs::rejected()`, so a dial that never
sent a packet is reported `rejected`. **That contradicts the contract.**
The TCP RPC path keeps the connect error's kind and conforms.

## 3. Problems

- **P1, silent setup failures.** Four of five Fang kinds log nothing when
  a session fails to set up, whatever the cause: wrong address family,
  peer down, peer not granting the target, Silver on the receiver, a key
  mismatch. The operator sees only resets in the application.
- **P2, unbounded plain-TCP logging.** A plain-TCP Fang whose target is
  down logs one line per connection attempt. Admission bounds concurrency
  (64), not the rate of failing connections, so a retrying client can
  grow the log without bound. The Stage42 incident (a 3.9 GB `/tmp`)
  came from the same pattern on another path.
- **P3, `fang active` cannot show failure.** A Fang whose every session
  fails looks the same as a healthy one.
- **P4, `rejected` for a local dial error** in discovery and sync,
  against the documented contract.
- **P5, QUIC Fang to a hostname peer** opens, then fails every session
  with an unbounded per-connection log line, while TCP Fangs, discovery
  and sync resolve the same address.

## 4. Options

### L1. Rate-limited setup-failure log per Fang (fixes P1, P2)

Each Fang kind logs a failed session **setup** with the Stage42
`RateLimitedLog` (first event, then at most one line per 10 s per Fang,
with the count of suppressed events): Fang id, peer, and a coarse class:

- `dial`: no connection was made (local socket error, refused, no route,
  no answer within the read window);
- `handshake`: the peer was reached but the session was not established
  (TLS, OPEN/ACK, signature, the receiver's refusal, which includes no
  Target Policy grant and Silver; the initiator cannot tell those apart,
  by design);
- for plain TCP: `target` (the target refused or did not answer).

To classify, `connect_v3_on` and the QUIC RPC path keep the error kind
for the dial step (as the TCP path already does) and stay
`hs::rejected()` for everything after it. **Only the initiator's own log
changes; nothing new is sent on the wire, and the receiver still learns
nothing.** Errors after a session is established (application resets,
EOF) are not logged: they are normal application behaviour.

Plain TCP moves its three `eprintln!`s onto the same limiter. That bounds
its log (P2) and changes its output from one line per failure to one line
per 10 s per Fang plus a count.

### S1. Contract-conformant discovery and sync status (fixes P4)

`rpc_quic` keeps the dial error's kind, so a local `endpoint.connect`
failure maps to `unreachable` through the existing `failure_status`.

- **S1a (recommended):** use the existing `unreachable`; no new status, no
  CLI contract change, no consumer change (`auto_sync_gate.py` counts
  `rejected` only for real refusals).
- **S1b:** a new status `dial_failed`. More precise, but changes the
  documented vocabulary that operators and gates read.

### C1. Session counters in `fang list` (fixes P3)

Add to each Fang's entry: `sessions_failed` (setup failures since the
Fang opened) and `last_failure` (`{"class": "dial"|"handshake"|"target",
"seconds_ago": N}` or `null`). Additive JSON fields: existing consumers
ignore unknown fields (`fang_listed` in the Stage42 tool and the gates
match on `local`, `transport`, `id`). The counter is one shared atomic per
Fang plus a small mutex for the last failure, updated only on failure.

- Alternative **C1-min**: `sessions_failed` only.
- Alternative **C0**: no counters; the log line (L1) is the only signal.

### H. QUIC Fang server address (fixes P5)

- **H1 (recommended):** resolve the peer address once when the QUIC Fang
  opens, taking the first address as discovery and sync do (S44-D4), and
  fail the open (`QUIC_FANG_FAILED`, with the reason) if it does not
  resolve. The per-connection parse and its log line go away. A DNS
  change is picked up when the Fang is reopened, as for a relay address.
- **H2:** refuse a QUIC Fang to a hostname peer at open, with a clear
  error. Visible, but leaves QUIC inconsistent with TCP, discovery and
  sync.

### Not proposed

- A `degraded` value for `state`. `state` is documented as the listener's
  state, and a threshold for "degraded" would be arbitrary; C1 gives the
  facts.
- Distinguishing the receiver's refusal reasons on the initiator. The
  receiver deliberately answers every refusal the same way; changing
  that is a protocol and trust question.
- Logging established-session errors.

## 5. Recommendation

**L1 + S1a + C1 + H1.** L1 makes every setup failure visible in the log
without letting a failing client grow it; S1a brings discovery and sync
back to their documented contract with no vocabulary change; C1 lets
`fang active` answer "is this Fang working?" directly; H1 makes a QUIC
Fang treat a hostname peer as TCP, discovery and sync already do, and
fail at open rather than per connection. None changes the wire, trust,
Target Policy or any receiver behaviour.

## 6. Proposed Stage45 contract (for owner review)

- **M1, reproduction** (lab, disposable, no live wolves; the 50 MB log
  guard stays), on `main`:
  - V1: for each Fang kind (QUIC, encrypted TCP, plain TCP, routed QUIC
    hop, failover), a session that fails because the peer is down, and
    one that fails because the receiver does not grant the target. Today:
    reset, nothing logged (plain TCP: a line per attempt), `fang active`
    unchanged.
  - V2: a plain-TCP Fang to a dead target, 500 attempts: log growth.
  - V4: a QUIC Fang and a TCP Fang to a peer whose Pack address is
    `localhost:PORT`: the QUIC Fang's sessions fail with a line per
    connection, the TCP Fang works (P5). If `pack add` refuses the
    hostname, P5 is recorded as unreachable from the CLI.
  - V3: discovery refresh and sync over QUIC to a peer whose QUIC dial
    fails locally. Since Stage44 no address does that on its own; M1
    uses a test-only fault (a unit test injecting the `endpoint.connect`
    error), not a production switch.
  - M1 stops the stage if nothing reproduces.
- **M2, fix:** L1, S1a, C1, H1. Unit tests: classification (`dial` vs
  `handshake` vs `target`); the limiter bounds lines and reports the
  suppressed count; counters increment only on setup failure and appear
  in `fang.list`; `rpc_quic` local error → `unreachable`. Workspace
  tests, fmt, clippy, `scripts/ci-local.sh` full;
  `docs/V1_CLI_CONTRACT.md` gains the two `fang list` fields.
- **M3, regression:** the M1 scenarios ×3; Stage42 flood scenarios (one
  round; they share the accept loops and logging); Stage32 S2 P0, S3 P0,
  S4 P0, S7 P0 (the dial path changes); the routed-Fang, discovery,
  provider and sync gates (through `ci-local`). Every historical verdict
  must hold.
- **M4:** `KNOWN_LIMITATIONS` update and closure.

## 7. Risks

- **Classification on the dial path** touches `connect_v3_on`, which
  every QUIC session uses. Only the *local* error kind is kept; the wire
  is unchanged. M3 reruns the QUIC cells.
- **Log volume:** one line per 10 s per Fang per class at most; the
  limiter is the Stage42 one.
- **Counters under flood:** one relaxed atomic increment per failure; no
  allocation per session.
- **A plain-TCP operator relying on per-connection lines** loses them;
  the count in the rate-limited line replaces them. Stated.

## 8. Owner decisions

| ID | Decision | Options | Recommended |
|---|---|---|---|
| S45-D0 | Approve Stage45 as scoped (§6) | approve, amend, reject | approve |
| S45-D1 | Setup-failure logging | L1 rate-limited per Fang, classes `dial`/`handshake`/`target`; per-event; none | L1 |
| S45-D2 | Plain-TCP per-connection lines | move onto the limiter; keep as is | limiter |
| S45-D3 | Discovery/sync local dial error | S1a `unreachable`; S1b new `dial_failed` | S1a |
| S45-D4 | `fang list` fields | C1 `sessions_failed` + `last_failure`; C1-min count only; C0 none | C1 |
| S45-D5 | QUIC Fang to a hostname peer | H1 resolve once at open; H2 refuse at open; leave | H1 |
