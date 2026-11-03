# Stage39 M2 — target-connection liveness fix

## 1. Executive summary

**M2-PASS.** Both secure receivers now apply the session sockets'
existing kernel liveness bounds to their target connection. The change is
11 production lines in two files, no new constant, and no `TCP_NODELAY`
change.

The targeted regression reran the Stage39 M1 matrix in the sandbox on
the candidate build, with M1's own tooling and pre-declared protocol.

- **Fault runs:** in all 16, Y released its session lease and target
  socket **55.9–80.9 s** after the Y ↔ D failure, well inside the
  declared 150 s bound. In M1, every equivalent run persisted beyond
  390 s.
- **Requester's view:** ECONNRESET every time, never a clean EOF.
- **The critical encrypted-TCP upload cell** (3 runs) now releases at
  80.1–80.9 s, even though A's outer connection has already gone (about
  61 s).
- **Healthy controls (6):** idle, upload and download on both hops kept
  their sessions for the full 300 s, with no early timeout, and released
  on client close.
- **Resources:** file descriptors returned to baseline in all 22 runs.
- **Existing checks:** the workspace tests (413), fmt, clippy, the
  locked release build, the Python unit tests (64), the acceptance suite
  (12) and the routed-Fang lab gate (12) all pass.

## 2. Commits

| | |
|---|---|
| Baseline | `main` `756f238ee7ac82ef89e725d3a560b8563f3ec00a` (#110 merged) |
| Candidate (fix and unit test) | `3f1477a25a7c347e319f1dbbf0bd5dc79d5e2886` |
| Candidate `werewolfd` (release, `--locked`) | `351db99f164dce10a97a029dea2ea6b39cbb12887dd5c06719c2ce893f7122fd` |
| `werewolfctl` | `24de9943…3d22` (unchanged) |
| Unchanged | `Cargo.lock`; tokio 1.52.3; quinn 0.11.12 and quinn-proto 0.11.18; S3-P2 EXCLUDED; Stage38 CLOSED; `v1.0.0-rc.2` (tag object `ec10180`) |

## 3. Root cause

This was established in [M1](STAGE39_M1_LIVENESS_REPRODUCTION.md). The
receiver's target connection, which on a relay is the WAN hop to the
destination, had keep-alive off and no `TCP_USER_TIMEOUT`, unlike every
session socket. A dead Y ↔ D path was therefore never detected while
idle, and was bounded only by `tcp_retries2` with data in flight. The
forwarding task stayed blocked on that socket, and held its authority
lease, even after the outer connection had gone.

## 4. The production diff

```diff
--- crates/werewolfd/src/transport/tcp_encrypted.rs
-fn configure_liveness(stream: &TcpStream) -> io::Result<()> {
+pub(super) fn configure_liveness(stream: &TcpStream) -> io::Result<()> {
@@ server_handshake
     .map_err(|_| hs::rejected())??;
+    // The target connection gets the session sockets' kernel liveness bounds
+    // (not their TCP_NODELAY). On a relay it is the WAN hop to the
+    // destination: without them a dead path holds the session and its lease
+    // until the kernel's retransmission limit (Stage39 M1). Fails closed:
+    // no publication, no ACK.
+    configure_liveness(&remote).map_err(|_| hs::rejected())?;
     // Publication and every subsequent submission share the authority gate.
     lease.publish()?;

--- crates/werewolfd/src/transport/quic.rs
@@ server_handshake
     .map_err(|_| hs::rejected())??;
+    // As on the encrypted-TCP receiver: kernel liveness bounds on the target
+    // connection, fail closed before publication and the ACK (Stage39 M1).
+    super::tcp_encrypted::configure_liveness(&target).map_err(|_| hs::rejected())?;
     lease.publish()?;
```

Plus one unit test (section 7).

## 5. Shared configuration design

The existing `configure_liveness` (keep-alive 30 s + 10 s × 3,
`TCP_USER_TIMEOUT` 60 s, from the existing constants) becomes
`pub(super)` in the `transport` module, so the QUIC receiver calls the
same function. It is not duplicated and not extracted.

`configure_session_socket` (which also sets `TCP_NODELAY`) is **not**
used for targets, so target sockets keep their Nagle behaviour.

## 6. Authorization and publication invariants

The call sits after `lease.connect_poll(…)` returns the authorized target
connection, and before `lease.publish()` and the ACK, in both receivers.
A configuration failure returns the generic rejection:

- the lease is not published;
- no ACK is sent;
- the target connection is dropped with the function's locals;
- the surrounding `select!` on `lease.cancelled()` is unchanged.

These are untouched:
- Pack authentication, exact-SPKI binding and the signed OPEN;
- Silver, admission, Target Policy and replay;
- the lease reservation and the lease-gated connect;
- publication order, framing and the forwarding loops;
- `arm_reset_on_close` and `disarm_reset_on_close` (still applied when
  forwarding starts);
- QUIC `TransportConfig`, plain TCP, and the wire and CLI contracts.

## 7. Socket-option unit tests

The new test is `target_liveness_keeps_nagle_and_reset_on_close`
(`transport/tcp_encrypted/liveness_tests.rs`). On a target-style
connected socket, after `configure_liveness`:

- **Options:** `SO_KEEPALIVE` on, `TCP_KEEPIDLE` 30 s, `TCP_KEEPINTVL`
  10 s, `TCP_KEEPCNT` 3, `TCP_USER_TIMEOUT` 60 s.
- **Unchanged:** `TCP_NODELAY` and linger are as before.
- **Reset-on-close still works on top:** `arm_reset_on_close` sets
  linger 0 and keeps the user timeout; `disarm_reset_on_close` restores
  linger without touching keep-alive.

The three existing liveness tests still pass.

**Both real receiver paths** were verified at runtime, not by source
search. The M1 method (`getsockopt` on a `pidfd_getfd` duplicate of the
live descriptor) read Y's actual target socket in all 22 sandbox runs,
for the encrypted-TCP hop (encrypted-TCP receiver) and the QUIC hop
(QUIC receiver), with the same result every time:

| `SO_KEEPALIVE` | idle / interval / count | `TCP_USER_TIMEOUT` | `TCP_NODELAY` | `SO_LINGER` |
|---|---|---|---|---|
| 1 | 30 / 10 / 3 | 60000 ms | 0 (unchanged) | on, 0 (reset-on-close armed) |

**Fail-closed path:** not unit-tested. Making `setsockopt` fail on a
freshly connected TCP socket would need a production fault-injection
hook, which the handoff asks to avoid. The path is the same `?`/rejection
pattern as the connect immediately before it.

## 8. Healthy-path results

All controls ran for 300 s with no fault, then the client closed:

| Hop, activity | Session for 300 s | Data | Y lease released on client close | fds after |
|---|---|---|---|---|
| tcp idle | yes, no early timeout | echo probe verified | at 300.0 s, with the close | 16 (baseline) |
| tcp upload | yes | 6.05 GB sent | 301.2 s | 16 |
| tcp download | yes | 11.87 GB received | 300.1 s | 16 |
| quic idle | yes | echo probe verified | 301.1 s | 16 |
| quic upload | yes | 11.91 GB sent | 301.0 s | 16 |
| quic download | yes | 10.21 GB received | 300.1 s | 16 |

- **Idle sessions survive past 60 s.** Healthy idle sessions survived
  300 s: keep-alive probes are answered, so 60 s of idleness ends
  nothing.
- **Half-close is unchanged:** the routed-Fang gate's "target finishing
  first ends a routed session promptly" passes for both hops (section
  16).

## 9–11. Fault results: blackhole, hard path loss, and every activity

Times are seconds after the fault. The protocol and tooling are
identical to M1's; M1's results are its historical baseline.

| Scenario | M1 (unchanged code): Y lease | **M2: Y lease released** | Y target socket gone | D socket gone | Requester |
|---|---|---|---|---|---|
| tcp blackhole idle ×3 | held > 390 s | **56.7, 55.9, 57.9** | same instant | same instant | ECONNRESET at 56.2, 55.6, 57.1 |
| tcp blackhole upload ×3 | held > 390 s | **80.9, 80.5, 80.1** | same instant | 61.1, 61.6, 61.3 | ECONNRESET at about 60.5 (see section 12) |
| tcp blackhole download | held > 390 s | **61.2** | 61.2 | 61.2 | ECONNRESET at 60.7 |
| tcp linkdown idle | held > 390 s | **57.0** | 57.0 | 57.0 | ECONNRESET at 56.0 |
| tcp linkdown upload | held > 390 s | **60.4** | 61.5 | 61.5 | ECONNRESET at 60.3 |
| tcp linkdown download | held > 390 s | **61.5** | 61.5 | 60.4 | ECONNRESET at 60.8 |
| quic blackhole idle | held > 390 s | **55.9** | 55.9 | 55.9 | ECONNRESET at 55.4 |
| quic blackhole upload | held > 390 s | **80.4** | 80.4 | 61.6 | ECONNRESET at 80.2 |
| quic blackhole download | held > 390 s | **61.3** | 61.3 | 61.3 | ECONNRESET at 60.8 |
| quic linkdown idle | held > 390 s | **57.1** | 57.1 | 57.1 | ECONNRESET at 56.5 |
| quic linkdown upload | held > 390 s | **60.5** | 60.5 | 61.5 | ECONNRESET at 60.2 |
| quic linkdown download | held > 390 s | **61.3** | 61.3 | 60.3 | ECONNRESET at 60.8 |

**Mechanisms observed** (from the `ss` timers):

- **Idle and download:** Y has nothing to send to D, so the target
  socket shows `keepalive` timers. It is torn down 55.9–61.5 s after the
  fault, about one user timeout after the last data from D. That is when
  `TCP_USER_TIMEOUT` ends the unanswered keep-alive sequence.
- **Upload:** Y has unacknowledged data (117–347 segments), so it shows
  retransmission timers.
  - **Blackhole:** torn down at 80.1–80.9 s. The kernel checks the user
    timeout when a retransmission timer fires, and with backoff (8) the
    first check past 60 s landed near 80 s in every run.
  - **linkdown:** 60.4–61.5 s.
- **Not a fixed deadline.** As the handoff warns, the release time is not
  a fixed 60 s from the fault. It depends on the socket's state, but it
  was bounded in every run, at 80.9 s or less against the declared
  150 s.

## 12. The encrypted-TCP upload lifecycle regression (mandatory)

The M1 condition reproduced exactly in all three runs:

- A's outer socket aborted at about 61 s (A's own zero-window user
  timeout);
- Y's outer socket disappeared at 61.3–62.2 s, and the requester saw
  ECONNRESET at about 60.5 s.

The difference from M1: Y's target socket now has a user timeout, so it
errored at 80.1–80.9 s, and the blocked forwarding task ended and
**released Y's lease, although its outer connection was already gone.**
In M1 the same lease was still held at 390 s. Y's fds returned to
baseline (16).

## 13. Lease and quota recovery

- **Y:** `inbound_authority.active` went to 0 at the release times above,
  in every fault run, and stayed 0 to the end of observation. The
  requester's per-peer quota at the relay is therefore restored. With
  one requester, the global count is the same number.
- **D:** its lease dropped with its own socket (55.9–61.6 s), as in M1.

## 14. Requester-visible failure semantics

Every fault run ended the requester's application connection with
**ECONNRESET**, never a clean EOF. The interrupted transfer is reported
as a failure.

For idle and download sessions this is new: M1's requester saw nothing
at all for 300 s. Y's error drops the outer session without
`close_notify`, A's inner session errors, and A's local socket, armed
reset-on-close, resets.

## 15. Resource measurements

| Measure | Result |
|---|---|
| Y fds | 16 at baseline, 16 after every one of the 22 runs. In M1 the fault runs ended at 17–18. |
| Y threads | 5–7 |
| Y RSS | 9.3–14.6 MiB |
| Daemons | all alive |

## 16. Test results

| Check | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | PASS |
| Lint | `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` | PASS |
| Rust tests | `cargo test --locked --offline --workspace` | PASS, 413 (412 before, plus 1) |
| Release build | `cargo build --release --locked --offline --workspace --bins` | PASS |
| Python unit tests | `python3 -B -m unittest discover -s tests/integration -p 'test_*.py'` | PASS, 64 of 64 |
| Acceptance | `python3 -B tests/integration/run.py` | PASS, 12 of 12 (at `3f1477a`) |
| Routed-Fang lab gate | `python3 -B tests/integration/routed_fang_gate.py` | PASS, 12 of 12 (at `3f1477a`): 64 MiB full duplex on both hops, target-first half-close on both hops, Silver on the relay, relay address change |
| Stage39 targeted sandbox | `tools/stage39/relay_target_liveness.py` (M1 tooling, unchanged) | 22 of 22 completed; 16 fault runs released, 6 controls healthy |

S3-P2 was not run, and is not a success criterion; it stays EXCLUDED.
The full Stage39 M3 campaign was not started.

## 17. Remaining risks and limitations

- **Sandbox only.** No WAN run (S39-D5 needs separate authorisation).
- **Release time varies within the bound.** It runs from about 56 s to
  about 81 s, depending on activity and backoff; not a fixed deadline.
- **A target unreachable for more than about 60 s ends the session,**
  for every secure session. That matches every other Werewolf hop.
  Keep-alive probes to target hosts are new observable traffic, one
  probe every 30 s of idleness.
- **Fail-closed is not unit-tested** (section 7).
- **Loopback application targets on direct sessions** take the same code
  path and are covered by the acceptance suite, not by fault cells.
- **Plain TCP is unchanged,** by decision S39-D3.

## 18. M3 regression proposal (needs separate authorisation)

- **Stage32:** rerun the network subset whose sessions use a target
  connection: S1, S2 and S3 at P0, P1, P2 (TCP hop), P4 and P5, with
  three runs each per the Stage32 rules. S3-P2's QUIC hop stays
  EXCLUDED.
- **Stage22R:** the D07 target-failure equivalents in the sandbox,
  direct and through a relay.
- **Lab gates:** the ten lab gates, and the Stage12 proofs.
- **Docs:** a `KNOWN_LIMITATIONS` update recording that relay-to-
  destination path loss is now bounded in the sandbox, not yet on a WAN.
- **Optionally afterwards:** a real-WAN relay-to-destination check
  (S39-D5).

## 19. Owner decision required

- **Merge this PR,** after review.
- **Authorise Stage39 M3** (section 18).
- **Optionally**, authorise the WAN check later.

Not authorised by M2: M3, real-WAN testing, reopening Stage38, tokio
promotion, and release certification.

## Evidence

Kept outside Git, in `target/stage39-m2/`:
- `MANIFEST.txt`: baseline and candidate SHAs, kernel and toolchain, and
  the SHA-256 of the binaries, scenarios, logs and every run record;
- `runs/*.json` and `runs/summary.md`;
- `acceptance.log`, `routed_fang_gate.log`;
- `build/`.

M1's evidence stays separately in `target/stage39-m1/`.
