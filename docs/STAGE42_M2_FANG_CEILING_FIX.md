# Stage42 M2 — Fang connection ceilings and accept back-off (fix)

**M2: PASS.** Every Fang kind now accepts through one bounded acceptor.
The M1 failures are gone. A Fang no longer dies on descriptor
exhaustion, the QUIC Fang no longer spins, and plain-TCP Fangs are
bounded. Every existing check passes.

**What M2 does not fix (stated plainly):** a flood on one Fang still
denies **that Fang's** other users while the flood lasts. Its 64 slots
are full, so a new connection is reset. Fangs to the same peer also still
share that peer's 64-session quota at the receiver. The ceilings protect
the daemon (descriptors, CPU, log, other Fangs to other peers) and make
the failure fast and explicit. They do not provide per-client fairness.
That needs per-client accounting and is not in this stage.

## 1. Change (`e9fb6e2`)

New `crates/werewolfd/src/transport/fang_accept.rs`, `FangAcceptor`:

| Rule | Value |
|---|---|
| Concurrent local connections per Fang | 64 (`FANG_CONNECTIONS`, = receiver `PEER_SESSIONS`) |
| Concurrent local connections across all Fangs | 1024 (`GLOBAL_FANG_CONNECTIONS`, = receiver `GLOBAL_SESSIONS`), one daemon-wide semaphore |
| Over either ceiling | accepted, then closed with RST (`arm_reset_on_close`) at once |
| Accept error (e.g. `EMFILE`) | wait 100 ms (`ACCEPT_BACKOFF`), keep listening |
| Logging of both events | at most one line per 10 s per Fang, with the count of suppressed events |

The slot is held by the connection's task and released when the session
ends. Every per-Fang permit is returned if the global permit fails.

Used by all five accept loops, replacing their own accept handling:

| Fang | Before | After |
|---|---|---|
| QUIC (`quic_fang.rs`) | logged every error, retried with no delay | `FangAcceptor` |
| encrypted TCP (`tcp_encrypted.rs`) | `accept()?`: Fang ended | `FangAcceptor` |
| plain TCP (`tcp_plain.rs`) | same | `FangAcceptor` |
| routed (`relay.rs`) | same | `FangAcceptor` |
| failover (`failover.rs`) | same | `FangAcceptor` |

No receiver quota, window, timeout or wire behaviour changes.

## 2. Unit tests (`transport/fang_accept/tests.rs`)

| Test | Shows |
|---|---|
| `per_fang_ceiling_resets_excess_and_frees_on_release` | with a ceiling of 2, the third connection is reset and never reaches a session; the held two stay open; releasing a slot admits the next |
| `global_ceiling_is_shared_across_fangs` | two Fangs sharing a global ceiling of 1: Fang B is reset while A holds it; no per-Fang permit leaks |
| `production_limits_match_the_receiver_quotas` | `new()` uses 64 and the shared 1024 semaphore |
| `repeated_events_are_logged_at_a_bounded_rate` | 10 000 events inside one interval log once, then one line after the interval |

The `EMFILE` path cannot be tested in-process without changing the test
process's descriptor limit. The lab covers it (§3).

## 3. The M1 scenarios on the M2 build (one run each)

Lab build `werewolfd` `b1a18eb0…223a`. Results in `target/stage42-m1/`,
files `m2-check-256-*.json` and `m2-check-64-*.json`. M3 repeats these
formally.

| Scenario | M1 (`main`) | M2 |
|---|---|---|
| fd:tcp, `nofile` 256 | Fang stopped listening after 158 connections, never recovered | keeps listening (599 connections made); legit echo after: **PASS** |
| fd:tcp-plain, 256 | stopped after 154, never recovered | keeps listening; legit after: **PASS** |
| fd:quic, 64 | **103% CPU**, log grew until stopped at 54 MB | **1.8% CPU**, log grew **188 bytes**; listening again after release; legit after: **PASS** |
| fd:tcp, 64 | stopped after 50, never recovered | keeps listening (541 connections); 1.8% CPU, 428 bytes of log; legit after: **PASS** |
| quota:tcp-plain | 422 echoed, **1017** descriptors | 64 echoed, **145** descriptors |
| quota:quic / quota:tcp | 64 echoed; legit during FAIL | 52 / 64 echoed; legit during PASS / FAIL (see note) |

In every scenario the daemon stayed up and the Fang stayed listed.

**Note on "legit during the flood".** Under encrypted and plain TCP, the
flood's 64 sessions hold all 64 slots, so the legitimate connection is
reset: the expected result, per the scope above. Under QUIC, 12 of the 64
flood sessions failed at the receiver's handshake admission (16
concurrent per peer), which freed slots, and the legitimate session got
one. That is incidental, not a guarantee.

**The M1 "UNEXPLAINED" item** (173 descriptors still open 5 s after the
plain-TCP flood): with the ceiling, the plain-TCP flood ends at 17
descriptors after 5 s. It is not reproduced here; M3 checks it again.

## 4. Existing checks

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --workspace --all-targets --all-features -D warnings`:
  PASS.
- `cargo test --workspace --all-features`: **418 passed**, 0 failed (414
  plus the 4 new tests).
- `scripts/ci-local.sh` full (`target/ci-local/20261109T195135Z`): **13 of
  13 PASS**. This includes acceptance (all transports, a 50 MiB transfer,
  daemon restart restoring Fangs), the routed-Fang gate (relay hops) and
  the service and discovery gates.

## 5. Next

M3: the M1 scenarios three times each, Stage32 S4-P0 and S4-P2 (50 opens
per run per transport), and the routed-Fang gate. Every historical
verdict must hold.
