# Stage37 — encrypted-TCP relay backpressure and shutdown diagnosis

**Result.** Outcome **A: correct behaviour under kernel backpressure.**
The `CLOSE-WAIT`-at-bound failure was reproduced on tokio 1.52.3 with
lifecycle tracing (run 17 of 30). Throughout, the relay was suspended in
a single encrypted frame write on a socket that the kernel never made
writable. That write could not complete because the TCP connection
toward A got no acknowledgement for 60 s: the retransmissions were lost
under the exponential retransmission backoff, and A's keepalive
segments, which carried the missing ACK, are discarded by standard TCP
segment validation. The kernel then ended the connection through
`TCP_USER_TIMEOUT` (ETIMEDOUT at 121.4 s), and the session released all
of its resources at once. No defect was found in write completion, EOF
propagation, shutdown ordering, cancellation or socket ownership, and no
writable notification was missed.

Diagnosis only (owner authorisation, Stage37 brief). No production code,
dependency, configuration, transport setting, harness or rule changed.
tokio stays at **1.52.3** on `main`, with RUSTSEC-2026-0386 accepted.
S3-P2 stays **EXCLUDED**. The frozen `v1.0.0-rc.2` tag and archive are
untouched, and no historical FAIL, EXCLUDED or verdict is reclassified.
wolf-a and wolf-b were not touched.

## 1. Baseline and environment

- `main` `4bcaaee3cbdbaba1394ca8721ae683cb0fe3f47c`, after PR #100
  (Stage35, merge `7a248ce`) and PR #101 (Stage36, merge `4bcaaee`).
- tokio 1.52.3 (`Cargo.lock`), quinn 0.11.12, quinn-proto 0.11.18.
- Host: Linux 6.12.113+deb13-amd64, rustc 1.96.0. The S3-P2 cell ran in
  the harness's unprivileged sandbox (`unshare -rmnp`), with the
  topology, impairments, seeds, 512 KiB payload and 120 s bound as
  `chaos.transfers` defines them.
- **Diagnostic build** (disposable, not for `main`): worktree
  `target/stage37/wt` at `4bcaaee` plus one local commit `6f8cc0e`
  (diff in `target/stage37/stage37-trace.diff`, SHA-256
  `5b9bbb4f…3832`), built with
  `cargo build --release --locked --offline --workspace --bins
  --features werewolfd/stage37-trace`. `werewolfd` `a65ca7e3…354f`;
  `werewolfctl` `24de9943…3d22`, byte-identical to the rc.2 build.

## 2. Call graph of the routed TCP hop

S3 routes a Fang from A to B through relay Y. On the TCP hop, the outer
session A–Y is encrypted TCP, and the inner session A–B runs inside it.

**A (client):**
`relay::run_routed_forwarder` accepts the application's connection and
spawns `relay::routed_session` on the Fang's cancellation set. For
`RouteHop::Tcp`, that function:

- opens the outer session with `connect_authenticated_tcp` and
  `client_handshake`;
- creates a 64 KiB `tokio::io::duplex` pipe (`near`, `far`);
- runs the outer pump, `secure_copy_client_side(far, outer)`, and the
  inner session, `inner_session(inbound, near)` (TLS to B, then
  `secure_copy_client_side(inbound, tls)`), in one `select!`.

**Y (relay)** and **B (destination)** run the same server path:

- `tcp_encrypted::run_fang_listener_with_ready` holds a `JoinSet` and
  spawns one child task per accepted connection;
- the child task runs the TLS accept (`AuthorityTcpStream` under
  tokio-rustls), then `handle_fang_pipe`, which calls `server_handshake`;
- `server_handshake` reserves the `SessionLease`, authorises the stream
  (`AuthorityTcpStream::authorize`), checks target policy and connects
  the target with `TcpStream::connect` through `lease.connect_poll`;
- then `secure_copy_server_side(stream, remote, key, lease)` runs. On Y
  the target is B's Fang listener; on B it is the echo.

**Inside `secure_copy_server_side`**, two directional futures run in the
same task under `tokio::try_join!`, raced against `lease.cancelled()`
with a `biased` `select!`:

- **`client_to_remote`:** `read_frame_or_end(fang_r)` decrypts each
  frame, then `remote_w.write_all` and `flush` forward it, where
  `remote_w` is an `AuthorityWriter` over the borrowed `WriteHalf`. At
  a clean end (TLS `close_notify`, read returns 0), it calls
  `remote_w.shutdown()`, which sends FIN to the target, and returns.
- **`remote_to_client`:** `remote_r.read` reads up to 1400 bytes, then
  `write_encrypted_frame(fang_w, …)` pads, encrypts, and makes one
  `write_all` and one `flush` of the frame. When `read` returns 0
  (target EOF), it calls `fang_w.shutdown()`, which sends TLS
  `close_notify` and FIN to the client, and returns.
- **The write path for each frame:** `WriteHalf<TlsStream>`, then
  tokio-rustls (which buffers TLS records until `flush` drains them),
  then `AuthorityTcpStream::poll_write`, then `lease.submit`, then
  `TcpStream::poll_write`. A frame's `flush` completes only when
  rustls has handed every byte of it to the kernel.
- **Ending:** when both futures finish, `disarm_reset_on_close` runs
  on the target socket and the function returns. Both sockets are
  dropped with it, and the TLS stream's drop closes Y–A.
- **Errors:** `try_join!` returns the first error and drops the other
  future. The target socket is still armed with zero linger, so it
  closes with a reset; the error ends the child task.

## 3. Ownership model

| Resource on Y | Owner | Released when |
|---|---|---|
| Y–A TCP socket (`AuthorityTcpStream` inside `ServerTlsStream`) | the listener's child task, through `secure_copy_server_side` (split into `fang_r` and `fang_w`) | the function returns, or the listener's `JoinSet` is dropped (task aborted) |
| Y–B TCP socket (`remote_stream`, borrowed halves) | the same task | the same |
| `SessionLease` (authority gate, cancellation) | the same task (the `select!` and both writers) | the same; `lease.cancelled()` ends the session early |
| TLS record buffer (rustls) and the current 1400-byte read | `remote_to_client` | the frame's `flush` completes, or the session ends |

There are no detached tasks, no spawned subtasks, and no other
references to either socket. Before ownership can be released, both
directions must finish: the client's `close_notify` must be read and
FIN sent to the target, and the target's EOF must be read, every frame
handed to the kernel, and `close_notify` and FIN sent to the client.
Otherwise an error or cancellation must end the session.

## 4. Encryption, forwarding and shutdown sequence

1. The application sends 512 KiB. A's inner session encrypts it and
   writes into `near`; the outer pump reads `far`, frames and encrypts
   it again, and writes to the A–Y socket.
2. Y's `client_to_remote` decrypts each outer frame and forwards the
   plaintext (the inner TLS byte stream) to B.
3. B decrypts the inner frames and writes to the echo, reads the echo's
   output, and encrypts it back toward Y.
4. Y's `remote_to_client` reads B's bytes (at most 1400 at a time), then
   encrypts and writes them toward A, one frame at a time.
5. A's kernel acknowledges the bytes it receives. A's outer pump
   decrypts them into `far`; the inner session decrypts them for the
   application.
6. When the upload ends, A sends `close_notify` and FIN. Y's kernel
   receives the FIN, and the Y–A socket enters `CLOSE-WAIT`; Y's
   `client_to_remote` reads the clean end and sends FIN to B.
7. B finishes its echo, and Y's `remote_to_client` reads B's EOF, but
   only once it has finished writing every earlier frame toward A.
   Then it calls `shutdown` (`close_notify` and FIN), and Y–A goes to
   `LAST-ACK`.
8. Both futures are done; the session ends, and both sockets drop.

`CLOSE-WAIT` on Y–A is therefore the expected state between steps 6
and 7. It lasts as long as Y still has echo bytes to hand to the
socket toward A.

## 5. Diagnostic instrumentation

Feature `stage37-trace` exists only in the disposable diagnostic build
(`crates/werewolfd/src/stage37_trace.rs` and hooks in
`authority/writer.rs` and `transport/tcp_encrypted.rs`).

- **Events:** `RELAY_SESSION_START`, `UPSTREAM_READ` (logged only if it
  was pending), `UPSTREAM_EOF`, `REMOTE_SHUTDOWN`, `REMOTE_READ`,
  `REMOTE_EOF`, `DOWNSTREAM_WRITE` (`PENDING` and `COMPLETE`),
  `DOWNSTREAM_SHUTDOWN`, `TASK_CANCEL_REQUESTED` (lease cancellation),
  `RELAY_SESSION_END` and `SOCKET_DROP` (the drop of the
  `AuthorityTcpStream`).
- **Below TLS, in `AuthorityTcpStream`:** `SOCK_WRITE` `WOULD_BLOCK` and
  `RESUMED` transitions, with the cumulative bytes the kernel accepted;
  an `SOCK_ACCEPTED` mark every 64 KiB; `SOCK_READ` EOF and errors; and
  `SOCK_SHUTDOWN` results.
- **Fields:** wall-clock and monotonic microseconds, session id, tokio
  task id, direction, operation, result, bytes requested and accepted
  (or poll count and wait time), `io::ErrorKind` name, and the time
  since the session's previous event.
- **Exclusions:** no key, nonce, plaintext, ciphertext, Den content or
  authentication material.
- **Bounds:** at most 300,000 lines per process, written to
  `/work/stage37-trace-<pid>.jsonl` inside the sandbox. Y wrote 5,427
  events (0.97 MB) in 30 runs.
- **Control flow:** each wrapper polls its inner future or I/O once per
  poll and returns the inner result unchanged. Without the feature the
  macros expand to the original expressions.
- **Perturbation:** a `Box::pin` per traced operation, and a
  synchronous file write per event.
- **Kernel side** (driver `target/stage37/s3_trace_capture.py`, which
  wraps the harness's own `transfers`): on every TCP-hop leg,
  `ss -tainmoH` (all states) on A, Y and B every 0.25 s, with wall-clock
  times, and headers-only `tcpdump` on A's and Y's links. Correlation
  with the trace is in `target/stage37/s37_analyze.py`.

**Campaign.** `S3:P2`, 10 rounds of seeds 2010, 2020 and 2030 (30 runs),
2026-11-02 10:43–11:21 UTC, plus a 1-round smoke check (3 runs). The
30 runs gave 29 PASS and **1 FAIL (run 17, seed 2020, TCP hop at the
bound)**, with 0 kernel tails. The TCP hop completed 25 times (median
21.9 s, max 63.7 s); 4 legs ended early with a clean reset. The QUIC
hop completed 10/10, 8/10 and 6/10 for seeds 2010, 2020 and 2030
(seed 2030 median 48.6 s), within the Stage34 description. These are
diagnostic results, not certification evidence.

## 6. Run 17: application event timeline

Times are seconds from the start of the TCP-hop leg; the session is
Y's `sid 17`. The complete listing is in `target/stage37/runs/run17-timeline.txt`.

| t (s) | Y application (trace) | Y–A socket (`ss`) |
|---|---|---|
| 1.46 | `RELAY_SESSION_START` | |
| 10.20–12.34 | frame write pending, socket `WOULD_BLOCK`, then `RESUMED` | |
| 12.34 | write pending again (`WOULD_BLOCK`, 520,752 B accepted so far) | |
| 12.85 | `UPSTREAM_EOF` (A's `close_notify`, 655,311 B forwarded to B); `REMOTE_SHUTDOWN` OK | → `CLOSE-WAIT` by 13.0 |
| 15.45, 22.05, 29.95, 30.28 | each pending frame write completes and the next blocks; writes block for 3.1 s, 6.6 s and 7.9 s | queue drains only as A's ACKs arrive |
| 30.28 | last `WOULD_BLOCK` (284 B of a TLS record offered and refused; 878,408 B accepted by Y's kernel so far) | |
| 30.28–121.43 | **one write pending throughout**; no other event | `CLOSE-WAIT`, never writable |
| 121.43 | `SOCK_WRITE` ERROR `TimedOut`, then `DOWNSTREAM_WRITE` ERROR, `RELAY_SESSION_END` ERROR `TimedOut`, `SOCKET_DROP` | socket gone |

`REMOTE_EOF` never occurred: B's last bytes were still waiting behind
the pending write. The harness's bound ended the leg at 120.0 s, 1.4 s
before the kernel abort.

## 7. Kernel and wire evidence for run 17

| t (s) | Y–A `bytes_acked` | send-q | `w` / `tb` | backoff | RTO | wire (captures on A's and Y's links) |
|---|---|---|---|---|---|---|
| 53.6–59.7 | 805,917 → 823,293 | 73,848 → 56,472 | 78,648 → 60,312 / 87,040 | 0 | ≈5 s | bursts of segments and ACKs; SACK recovery |
| 60.5 | 823,293 | 56,472 | 60,312 / 87,040 | 0 | 4.9 s | A SACKs 824742–829086; Y fast-retransmits 823294 |
| 65.6 | frozen | 56,472 | 60,312 / 87,040 | 1 | 9.9 s | RTO retransmission of 823294 seen on both links; **no ACK from A appears** |
| 75.5 | frozen | | | 2 | 19.7 s | `bytes_retrans` +1,448; **segment on neither link** |
| 95.3 | frozen | | | 3 | 39.5 s | `bytes_retrans` +1,448; **segment on neither link** |
| 100.96, 111.20 | frozen | | | 3 | | A's keepalives (seq 779418581 = `snd_nxt`−1, **ack 829086**) reach Y; Y answers with a duplicate ACK |
| 121.43 | | | | | | `TCP_USER_TIMEOUT` (60 s): ETIMEDOUT |

The three byte counts are kept separate:

- **Accepted by Y's kernel:** 878,408 B, from `SOCK_WRITE`.
- **Acknowledged to Y:** 823,293 B, from `bytes_acked`.
- **Received by A's kernel:** 829,085 B (`bytes_received`). A's
  keepalive ACK confirms 829,086 in sequence space.
- **Delivered to the application:** 480,784 of 524,288 plaintext bytes,
  in order.

The 5,792 bytes A had received beyond Y's acknowledged point were never
credited to Y.

What this shows:

- **Never writable:** from 55 s, `w` = 60,312 is above two-thirds of
  `tb` (58,027), Linux's write-readiness threshold
  (`sk_stream_is_writeable`). The kernel would not signal write
  readiness, and with no ACK progress, the queue could not shrink.
- **A missed wakeup is excluded:** with an ACK frozen from 59.7 s, no
  queue drain was possible. A wakeup would have changed nothing,
  because the hole at 823,294 holds back every later byte, including a
  FIN.
- **Where the losses were:** the 65.6 s retransmission reached A's link,
  but A's ACK appears on neither capture. The 75.5 s and 95.3 s
  retransmissions were counted by Y's kernel but appear on neither
  capture. The captures sit on each namespace's own interface, after
  the egress queue where netem drops packets, so these losses happened
  on the impaired links. The drop point is inferred: the netem
  counters were not read.
- **Why the keepalives did not help:** a keepalive probe deliberately
  carries `snd_nxt`−1. To Y it is an unacceptable segment, which TCP
  answers with an ACK and then discards before its ACK field is
  processed (RFC 9293, segment arrival; Linux `tcp_validate_incoming`).
  So A's two keepalives told Y nothing.
- **How it ended:** the next retransmission was due at about 135 s.
  Before that, `TCP_USER_TIMEOUT` (60 s since the last ACK at 59.7 s,
  `configure_liveness`) ended the connection at 121.4 s.

## 8. All `CLOSE-WAIT` intervals

Across the 33 traced runs, 24 runs had a `CLOSE-WAIT` interval, and 10
of them lasted 10 s or more (the smoke run 1; campaign runs 7, 9, 11,
13, 16, 17, 21, 22, 26). In those 10:

- 905 samples at 0.25 s. In 900, the application was blocked in a frame
  write (`WOULD_BLOCK` not yet `RESUMED`). In the other 5, it was
  waiting for B's next bytes.
- In 0 samples was Y–A writable.
- In no run was there a sample where Y–A was writable while a write
  stayed blocked 0.5 s or more afterwards.
- Y's socket to B often held unread data (`Recv-Q` > 0; for example,
  47 of 49 samples in run 13 and 53 of 56 in run 26). Data from B was
  waiting behind the blocked write toward A.
- 9 of the 10 ended when the blocked write completed, the remaining
  frames were written, B's EOF was read, and `shutdown` was sent. Run 17
  ended by the kernel abort.

Lifecycle latencies, over all sessions:

- **EOF to shutdown:** on Y (26 sessions that reached B's EOF), the
  socket shutdown completed at most **249 µs** after `REMOTE_EOF`. On B
  (50 sessions) it was at most 474 µs. No `shutdown` was ever pending.
- **End to drop:** the socket was dropped at most 84 µs after
  `RELAY_SESSION_END` on Y, and 197 µs on B.
- **Cancellation:** no lease cancellation (`TASK_CANCEL_REQUESTED`)
  occurred.
- **Errors:** the only errors were run 17's `TimedOut`, and run 6's
  `BrokenPipe` when forwarding to B. The latter was a clean early end
  of the leg, which the harness counted as a reset.

## 9. Why Y stays in `CLOSE-WAIT`

Y has received A's FIN, but it still owes A the rest of B's echo. It
cannot send its own FIN until every earlier frame has been handed to
the socket. That ordering is required: a FIN, or a TLS `close_notify`,
sent before the data would truncate the echo. Y holds at most:

- the unacknowledged send queue (`sndbuf`-bounded, about 60–130 KB
  here);
- one partially written TLS record and frame (at most 2,048 B of
  plaintext);
- the 1,400-byte read buffer;
- the Y–B receive queue (kernel-bounded).

During long `CLOSE-WAIT` intervals the socket is above the kernel's
write threshold, and only ACKs from A can bring it below. Under the
S3-P2 impairments a run of losses on both data and ACKs drives the
retransmission timer into exponential backoff. The base RTO there is
about 5 s, from the RTT variance accumulated earlier in the transfer.
The relay then waits, correctly, for the kernel.

## 10. Hypotheses

| Hypothesis | Supporting evidence | Contradicting evidence | Confidence |
|---|---|---|---|
| Kernel retransmission backoff | Run 17: no ACK progress for 60 s; backoff 1→3 and RTO 9.9→39.5 s; retransmissions counted but lost; the end is `TCP_USER_TIMEOUT` ETIMEDOUT. The other 9 long intervals all ended when the backlog drained with ACKs. | none | **Confirmed** for run 17; consistent with every traced interval |
| Correct pending asynchronous write | The trace shows one frame write pending from 30.28 s to the error; `WOULD_BLOCK` and `RESUMED` match the socket's state in every interval | none | **Confirmed** |
| Missed writable notification | none | Y–A was never writable while a write was blocked (0 of 905 samples, 0.25 s); in run 17 the queue could not shrink without ACKs; writes resumed as queues drained | **Rejected** at 0.25 s resolution |
| Incorrect EOF propagation | none | `UPSTREAM_EOF` is followed by `REMOTE_SHUTDOWN` OK in every session that read A's clean end; B's EOF reaches `REMOTE_EOF` once earlier frames are written; Y–B data waits behind the write, as designed | **Rejected** |
| Delayed shutdown | none | Shutdown completed at most 249 µs after `REMOTE_EOF` on Y; it was never pending | **Rejected** |
| Task cancellation or ownership defect | none | No cancellation fired; the socket dropped at most 84 µs after session end, run 17 included; the task, which owns both sockets, ended on the error; no detached tasks; `SOCKET_DROP` occurred for every session | **Rejected** for the observed paths. Lease cancellation and listener abort were not exercised. |
| Encrypted framing or buffering issue | none | A frame's `flush` completes only when rustls has emptied its buffer to the kernel. In run 17, the only bytes held above the kernel were one refused 284-byte TLS write and the frames not yet read from B. Data at A matched in order. | **Rejected** |

## 11. Classification

- **Root cause: PROVEN** for the traced failure (run 17) at the evidence
  level stated above. The drop points of the lost packets are inferred
  from the captures, not from qdisc counters. For the untraced FAILs
  (Stage35 run 28; Stage36 uninstrumented A block 2 run 10 and block 3
  run 12), it is **probable**: they show the same state and queue
  levels, but no lifecycle trace.
- **Outcome A:** correct behaviour under kernel backpressure. No
  application lifecycle defect was demonstrated.
- **Tokio:** the failure reproduced on 1.52.3, the production version,
  with tracing. Nothing in the mechanism depends on tokio's scheduler
  beyond correct write readiness, which was observed. Stage36 found no
  material TCP difference between 1.52.3 and 1.54.1. 1.54.1 was not
  traced here.

## 12. Security and resource-bound assessment

- **Resources stay bounded during a stall.**
  - Y keeps one task, two sockets, one lease, the kernel send queue
    (bounded by `sndbuf`), at most one frame and TLS record, and a
    1,400-byte buffer.
  - The stall cannot outlast the kernel limits already set
    (`TCP_USER_TIMEOUT` 60 s since the last ACK, plus keepalive).
  - In run 17, all of it was released 84 µs after the error.
  - No leak or crash was reported by the harness's after-settle checks.
- **Failure semantics:** the session ends with an error, not a clean
  end. Y's target socket is still armed with zero linger, so B sees a
  reset, and A's application sees an incomplete transfer, never a
  truncated "success". Data delivered before the stall was correct and
  in order.
- **Security:** no secret material is involved, and nothing in the
  mechanism touches Fang, Pack, Pelt, Silver, governance,
  authentication or authorization.

## 13. Recommendations

1. **Stage38 (proposal only, owner decision):** comparative
   certification criteria for S3-P2's TCP hop, for example a
   candidate's FAIL and tail rates against a concurrent 1.52.3 control
   in alternating blocks. A rule could cover a stall that has this
   record's signature (relay blocked on a non-writable socket, ACK
   progress frozen, ended by the kernel), but only with trace or
   capture evidence. Stage37 changes no rule; the kernel-tail rule,
   the F3 rule, the 120 s bound and the 512 KiB payload stand.
2. **No application fix is proposed.** No defect was demonstrated, and
   the relay cannot complete a write that the kernel cannot deliver.
3. **Liveness options, if the owner wants them** (Outcome C; each needs
   architectural review, none is recommended now):
   - **Shorter or adaptive kernel limits:** a shorter
     `TCP_USER_TIMEOUT`, which would fail stalls sooner, not complete
     them.
   - **Linear timeouts for thin streams** (`TCP_THIN_LINEAR_TIMEOUTS`):
     these apply only below 4 segments in flight; run 17 had 5.
   - **Leave the kernel as is.** The stall is a property of a path with
     about 10% loss per impaired egress, not of Werewolf.
4. **tokio:** kept separate. 1.54.1 stays accepted for further
   certification only; promotion needs its own decision.
   RUSTSEC-2026-0386 stays accepted.
5. **S3-P2 stays EXCLUDED** (quinn-proto 0.11.18, Stage34). No rc.3.

## 14. Not proven

- **Drop points:** where the three lost packets in run 17 were dropped
  is inferred (capture position after each egress qdisc); the netem
  drop counters were not read.
- **Earlier FAILs:** the untraced FAILs of Stage35 and Stage36 are
  attributed to the same mechanism by similarity only.
- **Sampling limits:** `ss` sampling at 0.25 s can miss writable
  moments shorter than that. Within this design, the trace's
  `WOULD_BLOCK` and `RESUMED` events record only the transitions that
  tokio observed.
- **Untested paths:** lease cancellation, listener abort and Silver
  transitions during a stall did not occur and were not tested.
- **One build:** only tokio 1.52.3 was traced. One FAIL in 30 runs
  gives no rate estimate.
- **Perturbation:** the tracing added an allocation per operation and
  a file write per event. Run 17's mechanism, loss and backoff, is
  independent of application timing.
- **Base RTO:** why the base RTO reached about 5 s was not analysed
  beyond the RTT variance that `ss` reported.

## 15. Evidence

Outside Git history, under `target/stage37/`:

- `stage37-trace.diff`, `wt/` (commit `6f8cc0e`), `build-trace/release`
  (binaries) and `build-check/` (the same source built without the
  feature, compile check only);
- `s3_trace_capture.py` and `s37_analyze.py`;
- `runs/c10/`:
  - `cell.json` (SHA-256 `3cf3b779…d56c`);
  - per-run pcaps `run*-tcp-link-{a,y}.pcap` and `run*-tcp-ss.json`;
  - traces `stage37-trace-62.jsonl` (Y, SHA-256 `23284f53…33b2`) and
    `stage37-trace-56.jsonl` (B);
  - daemon logs;
- `runs/c10-analysis.txt`, `runs/run17-timeline.txt`, `runs/smoke/`
  and `runs/smoke-analysis.txt`.

Stage36 reference evidence: `target/stage36/`.
