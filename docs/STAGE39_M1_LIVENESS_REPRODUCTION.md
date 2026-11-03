# Stage39 M1 — relay-to-destination liveness reproduction

## 1. Executive conclusion

**M1 conclusion: CONFIRMED.** On unchanged production code, a silent or
routing failure of the relay-to-destination (Y ↔ D) path leaves the
relay's session holding its authority lease and its target socket.
This happened in every one of 14 valid fault runs:

- 2 outer transports, 2 fault types, 3 activity states, plus 2 extra
  repetitions of the decisive cell;
- the lease stayed past the pre-declared 150 s bound, throughout the
  300 s window, and still 90 s after the requester's application had
  closed.

The attribution is direct:

- **The cause:** Y's target socket runs with keep-alive off and no
  `TCP_USER_TIMEOUT`, verified at runtime on the live descriptor. Y's
  other sockets have both.
- **The positive control:** in the same runs, D's accepted socket from
  Y, which does have them, detected the same failure and closed in
  55.8–79.7 s.
- **The other path stayed healthy:** A ↔ Y answered in every sample.

The healthy controls released normally. A second, related
forwarding-lifecycle finding is in section 11.

M2 is **not** implemented here, and is not authorised.

## 2. Repository baseline

| Item | Value |
|---|---|
| `main` | `7b9f2c1fa45c580ea5fc57ea626d4387f3cec3eb` (#109 merged) |
| Binaries | built from `main`: `werewolfd` `fe66ba76…05c5` (byte-identical to the rc.2 certification build), `werewolfctl` `24de9943…3d22` |
| Toolchain | rustc 1.96.0; tokio 1.52.3, quinn-proto 0.11.18 (unchanged) |
| Host | Linux 6.12.113+deb13-amd64; `tcp_retries2` 15, `tcp_keepalive_time` 7200 (unchanged; no sysctl was modified) |
| Release | tag `v1.0.0-rc.2` unchanged (object `ec10180`) |

## 3. Hypothesis (from the Stage39 survey, section 7.1)

The receiver's target connection has no keep-alive or user timeout, so,
on a relay, a dead Y ↔ D path would leave routed sessions:

- **idle:** until the requester ends them;
- **with data in flight:** for up to the kernel's retransmission limit,

much longer than the 60 s bound every other hop has.

## 4. Source-code evidence

- `configure_session_socket` (nodelay, keep-alive 30 s + 10 s × 3,
  `TCP_USER_TIMEOUT` 60 s) is applied to:
  - accepted encrypted-TCP sockets ([`tcp_encrypted.rs:124`](../crates/werewolfd/src/transport/tcp_encrypted.rs));
  - outbound session sockets (`:507`).
- The target connection, `lease.connect_poll(TcpStream::connect(…))`,
  gets none, on either transport:
  - encrypted TCP: `tcp_encrypted.rs:251`;
  - QUIC: [`quic.rs:338`](../crates/werewolfd/src/transport/quic.rs).
- That connection is only armed for reset-on-close (`arm_reset_on_close`:
  `SO_LINGER` 0).
- The relay's forwarding (`secure_copy_server_side`) has no application
  idle timeout. It ends when either direction ends or errors, or on
  lease cancellation.

## 5. Test environment

The Stage32 sandbox, `tests/integration/chaos.py`:
- `unshare -rmnp`, so no host privileges and no host network change;
- a private read-only root, disposable Dens;
- daemons from the build above;
- the namespaces, veths and processes vanish with the sandbox.

**Driver:** [`tools/stage39/relay_target_liveness.py`](../tools/stage39/relay_target_liveness.py).
D runs test services ([`s39_targets.py`](../tools/stage39/s39_targets.py)),
and A runs an application client
([`s39_client.py`](../tools/stage39/s39_client.py)) that never closes
until told to. Runs took 2026-11-03, 399–401 s each.

## 6. Namespace topology

```text
A 10.32.1.2 ──vA/rA── R (router) ──rY/vY── Y 10.32.3.2
                       │
                       └──rB/vB── D 10.32.2.2      (the sandbox names D "b")
```

- The routed configuration is exactly `chaos.transfers`'s:
  - Pack entries both ways;
  - Y's Target Policy allows `10.32.2.2:8443` for `wolf-a`;
  - D allows the test service for `wolf-a`;
  - `route.set wolf-b --via wolf-y --hop tcp|quic` on A;
  - a Fang on A at `127.0.0.1:17001`.
- The real authenticated Stage26 path: an outer session A → Y (encrypted
  TCP or QUIC), a plain-TCP target connection Y → D:8443, and an
  encrypted-TCP inner session A ↔ D inside it.
- **D's link carries only Y ↔ D traffic** in this configuration, so
  impairing it cannot touch A ↔ Y.

## 7. Baseline controls

| Check | Result |
|---|---|
| The session is established and carries its activity (idle: a 64-byte echo probe verified; upload and download: more than 1 MiB moved) before the fault | 16 of 16 |
| Socket identity: exactly one Y socket `10.32.3.2:* → 10.32.2.2:8443` owned by Y's daemon PID; exactly one D socket `:8443 ← 10.32.3.2`; for the TCP hop, one Y socket `:8443 ← 10.32.1.2` | 16 of 16 |
| A reaches Y (ping) in every sample after the fault | 16 of 16 runs, every sample |
| Y reaches D after the fault | 0 samples in every fault run; every sample in the controls |
| Healthy control, `tcp:none:idle` and `quic:none:idle` | the session stays (idle by design); when the client closes, Y's and D's leases drop to 0 within the same 1 s sample, and Y's target socket leaves TIME-WAIT 60 s later; fd count back to baseline (16) |

**Runtime socket options,** via `getsockopt` on a `pidfd_getfd`
duplicate. The daemon's socket is only read; the duplicate is closed
afterwards. Identical in every run:

| Socket | `SO_KEEPALIVE` | idle / interval / count | `TCP_USER_TIMEOUT` | `TCP_NODELAY` | `SO_LINGER` |
|---|---|---|---|---|---|
| **Y → D (target)** | **0** | 7200 / 75 / 9 (kernel defaults, unused) | **0** | 0 | on, 0 (reset-on-close) |
| D ← Y (accepted) | 1 | 30 / 10 / 3 | 60000 ms | 1 | off |
| Y ← A (outer, TCP hop) | 1 | 30 / 10 / 3 | 60000 ms | 1 | off |

## 8. Fault injection

Each fault was injected 5 s after the session was ready:

- **Blackhole:** `tc qdisc add dev vb root netem loss 100%` in D's
  namespace, and the same on `rb` in the router's. Packets vanish in both
  directions; nothing is closed or stopped.
- **Hard path loss (`linkdown`):** `ip link set rb down` in the router.
  The router loses its route to D, so Y's packets meet ICMP unreachable
  errors, not silence. Linux treats these as soft errors on an
  established socket, so they do not abort it.

## 9. Scenario results

Times are seconds after the fault. "Lease" is Y's
`inbound_authority.active` count from `status`. Every fault run:

| Scenario | Valid | Y target at 150 s | Y target timer in window | D socket gone | Y lease at 150 s | Y lease 90 s after client close | Y target at the end | Client |
|---|---|---|---|---|---|---|---|---|
| tcp blackhole idle ×3 | yes | ESTAB | none | 56.5, 55.8, 57.2 | 1 | 1 | FIN-WAIT-1, retransmitting (backoff 4) | no event until closed |
| tcp blackhole upload | yes | ESTAB | retransmit (609 unacked, backoff 8) | 61.1 | 1 | 1 | ESTAB, retransmitting | ECONNRESET at 60.3 |
| tcp blackhole download | yes | ESTAB | none | 79.7 | 1 | 1 | FIN-WAIT-1 | no event |
| tcp linkdown idle | yes | ESTAB | none | 57.5 | 1 | 1 | FIN-WAIT-1 (backoff 8) | no event |
| tcp linkdown upload | yes | ESTAB | retransmit (218 unacked, backoff 10) | 60.8 | 1 | 1 | ESTAB | ECONNRESET at 60.4 |
| tcp linkdown download | yes | ESTAB | none | 60.8 | 1 | 1 | FIN-WAIT-1 | no event |
| quic blackhole idle | yes | ESTAB | none | 57.4 | 1 | 1 | FIN-WAIT-1 | no event |
| quic blackhole upload | yes | ESTAB | retransmit (597 unacked, backoff 8) | 61.7 | 1 | 1 | ESTAB | no event |
| quic blackhole download | yes | ESTAB | none | 61.1 | 1 | 1 | FIN-WAIT-1 | no event |
| quic linkdown idle | yes | ESTAB | none | 56.0 | 1 | 1 | FIN-WAIT-1 | no event |
| quic linkdown upload | yes | ESTAB | retransmit (452 unacked, backoff 10) | 61.5 | 1 | 1 | ESTAB | no event |
| quic linkdown download | yes | ESTAB | none | 61.0 | 1 | 1 | FIN-WAIT-1 | no event |

- **Classification:** all 14 fault runs are PERSISTED_BEYOND_BOUND, and
  neither control is invalid.
- **No variation:** the three repetitions of the decisive cell agree to
  within 1.4 s on D's teardown, and exactly on Y's persistence.
- **Faults compared:** the two fault types behave alike for persistence.
  The routing errors of `linkdown` did not abort Y's socket.

## 10. Socket-level observations

- **Idle and download (Y has nothing to send to D):**
  - Y's target socket shows **no timer at all** for the whole window, so
    no keep-alive probe is ever sent and the kernel never learns the path
    is dead.
  - D's configured socket shows `keepalive` timers and closes at
    55.8–61.1 s. For tcp download it was 79.7 s, because data was in
    flight toward Y and the user timeout governed.
- **After the client closes:** Y sends a FIN to D, which is never acked.
  The socket moves to FIN-WAIT-1 and retransmits with exponential
  backoff (backoff 4 after 90 s in the blackhole runs, 8 under
  `linkdown`).
- **Upload (Y has data for D):** Y's target socket retransmits, with
  218–609 segments unacked and backoff 7–11 at the end. Without
  `TCP_USER_TIMEOUT`, only `tcp_retries2` (15 here) bounds it. With
  exponential backoff that is on the order of 15 minutes, a projection
  this run did not observe.
- **What was not observed:** no run observed the eventual kernel abort.
  The persistence time beyond 390 s is a projection from `tcp_retries2`
  and backoff, not a measurement.

## 11. Forwarding-task behaviour

- **The task blocks on the target.** Y's `secure_copy_server_side` runs
  both directions under `try_join!`, and neither waits on anything but
  the target connection here:
  - `remote_to_client` waits in `remote_r.read`, and D never answers;
  - `client_to_remote`, once A's data stops being forwardable, blocks in
    `remote_w.write_all`.
- **Second finding (TCP-hop upload runs):** A's outer socket aborted
  about 61 s after the fault. A's own `TCP_USER_TIMEOUT` (Linux aborts
  a zero-window stall after the user timeout) did this, because Y had
  stopped reading. Y's outer socket then disappeared from `ss` (61.1 s
  and 61.9 s). **Yet Y's lease and target socket stayed until the end:**
  the forwarding task never polls the outer socket again while blocked
  on the target, so it does not see its outer connection fail.
- **Under the QUIC hop:** no outer abort occurs at all (QUIC flow
  control has no zero-window abort, and keep-alives continue).
- **Attribution of the second finding:** it is also caused by the target
  socket's missing liveness. A bounded target socket would error, and
  end the task within its bound. It is recorded separately because it
  shows that the outer side's own bound does **not** release a relay
  session that is blocked on its target.

## 12. Session and quota cleanup

- **Y:** in every fault run Y's `inbound_authority.active` stayed 1 from
  the fault to the end of observation (300 s + 90 s). It is part of the
  requester's per-peer quota at the relay (`PEER_SESSIONS` 64, global
  1,024: [`authority.rs`](../crates/werewolfd/src/authority.rs)). So the
  quota was not restored within the observation.
- **D:** D's lease dropped to 0 when D's socket closed (55.8–79.7 s).
- **Resources:** Y's fd count stayed at baseline + 2 (TCP hop: the outer
  and target sockets) or + 1 (QUIC hop) at the end. Threads were
  unchanged (5) and RSS 9.4–13.8 MiB.

These are held resources, not a proven leak: the kernel abort will
eventually end them. The defect is the length of the hold, not
permanence.

## 13. Application-visible results

- **Idle and download:** the requester's application saw no EOF or
  error at any point in the 300 s window. A routed session to a
  destination that has been unreachable for 5 minutes looks open.
- **TCP-hop upload:** ECONNRESET at about 60 s, from A's own outer-socket
  user timeout, not from the relay or the destination.
- **QUIC-hop upload:** nothing; writes simply stop progressing.

## 14. Evidence for the defect

| Criterion (handoff section 10) | Evidence |
|---|---|
| A real Y → D target connection lacks the intended liveness configuration | runtime `getsockopt`: keep-alive 0, user timeout 0 (section 7) |
| The Y → D path is unavailable | `y_to_d` ping false in every sample after injection; `tc`/`ip` commands recorded |
| A → Y stays healthy | `a_to_y` ping true in every sample, every run; Y's outer socket keep-alive timers answered (idle and download) |
| Session and authority persist beyond the declared bound | Y's lease 1 and target socket present at 150 s, 300 s and 390 s, in 14 of 14 fault runs |
| Persistence is attributable to the target side's failure detection, not the client or the harness | the client stayed open by design and logged no early close; the same failure was detected by D's configured socket in 56–80 s; Y's target socket shows no keep-alive timer; the healthy controls release at once on client close |

**Not a harness artefact:** the controls released normally, identity was
verified per run, and no run was invalid. Neither the encrypted-TCP nor
the QUIC idle timeout is involved: the QUIC connection stays alive
through keep-alives.

## 15. Limitations

- **Sandbox only:** one host, namespaces and netem. A real WAN check
  needs separate authorisation (S39-D5).
- **The end of persistence was not observed:** observation stopped at
  390 s, so the kernel's eventual abort time (`tcp_retries2`) is a
  projection.
- **Aggregate only:** `status` reports Y's active-session count, not the
  per-peer count; with one requester they are the same.
- **Not tested:** plain TCP (excluded, S39-D3) and loopback application
  targets on direct sessions (the same code path, low exposure).
- **A distinction worth keeping:** `linkdown` produces routing errors
  that Linux treats as soft; a fault that produces hard errors (an RST
  from D) would end the session at once, as D07 shows.

## 16. Proposed M2 scope (needs separate authorisation)

A narrow correction, as already decided (S39-D1–D3):

1. **One shared function:** in `transport/mod.rs` (or the encrypted-TCP
   module), applied to the target socket right after `lease.connect_poll`
   returns, on both secure transports. It sets
   `configure_liveness`'s existing values (keep-alive 30 s + 10 s × 3,
   `TCP_USER_TIMEOUT` 60 s).
   - **No `TCP_NODELAY`:** target sockets keep their current Nagle
     behaviour.
   - **Order:** the setup failing ends the session like any other
     target-setup error, after `arm_reset_on_close`.
2. **Nothing else changes:**
   - authorization order;
   - the lease-controlled connect;
   - reset-on-close;
   - QUIC parameters;
   - forwarding loops;
   - plain TCP;
   - Stage34 and Stage37 rules;
   - every other constant.
3. **Tests:**
   - a unit test that a target socket from each listener carries the
     options;
   - this M1 matrix rerun on the fix, with the same declared protocol,
     plus a new expectation: Y's lease and target socket released within
     the bound plus `CLEANUP_WINDOW`, the requester sees a reset (never a
     clean EOF), and fds return to baseline;
   - regression on the affected Stage32 surfaces, the D07 equivalents and
     the existing suites and gates.
4. **The second finding (section 11)** needs no separate change: with a
   bounded target socket the blocked task errors and ends. M2's rerun
   checks this explicitly for the TCP-hop upload cell.

## 17. Owner decision required

- **Authorise M2** as scoped in section 16, or amend it.
- **Optionally**, authorise a WAN check later (S39-D5), after an M2
  sandbox PASS.

Not authorised by M1: any production change, M2, tokio promotion,
reopening Stage38, and release certification.

## Evidence

Kept outside Git, in `target/stage39-m1/`:
- `MANIFEST.txt`: the baseline, kernel, toolchain, the relevant
  sysctls, and the SHA-256 of the binaries, `campaign.scenarios`,
  `campaign.log` and every run record;
- `runs/*.json`: per run, the configuration, socket identity and
  options, the fault commands, 1 s samples of every socket of interest,
  leases on Y and D, Y's fd, thread and RSS counts, A↔Y and Y↔D
  reachability, and the client log;
- `runs/summary.json`;
- `build/` and `smoke/` (the topology check, not evidence).

Reproduction commands are in
[`tools/stage39/README.md`](../tools/stage39/README.md).
