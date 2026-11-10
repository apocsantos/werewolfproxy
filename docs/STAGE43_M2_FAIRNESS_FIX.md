# Stage43 M2 — Idle eviction and per-source share (fix)

**M2: PASS.** Both M1 scenarios are fixed for every Fang kind tested. A
Fang full of idle connections now admits a newcomer, and one source
address can no longer lock out another. The Stage42 flood results hold,
and every existing check passes.

**What M2 does not fix (stated plainly):**

- An **active** flood from loopback (the operator's own host) still fills
  a Fang and locks out its other local clients while it lasts. Loopback
  clients share one address, and active connections are never evicted.
  Survey §3: nothing can separate that flood from a legitimate client of
  the same user.
- Under pressure, a legitimate session idle for 60 s or more (an idle SSH
  session, say) can be ended to admit a newcomer. That is the trade the
  owner approved (S43-D1/D2).
- Fangs to one peer still share that peer's 64-session quota (S43-D4,
  recorded only).

## 1. Change (`faa1da2`)

`crates/werewolfd/src/transport/fang_accept.rs`:

| Rule | Value |
|---|---|
| Idle eviction | when a Fang is full, its longest-idle connection is ended (RST to its client) to admit the newcomer, if no byte has passed either way for **60 s** (`IDLE_EVICT_AFTER`). A Fang under its ceiling never evicts |
| Per-source share | a non-loopback source address holds at most **16** of a Fang's 64 slots (`SOURCE_SHARE`). At its share, a source may reclaim only **its own** idle connection; otherwise it is reset. Loopback (including IPv4-mapped loopback) is exempt; an IPv4-mapped address counts with its IPv4 form |
| Newcomer waits for the evicted slot | up to 1 s; a connection already being evicted is never chosen twice |
| Global ceiling (1024) | unchanged: reset only, no eviction across Fangs |
| Non-loopback bind (S43-D5) | one startup line: the Fang is reachable by any host that can reach that address |
| Logging | evictions and share resets at the Stage42 bounded rate (one line per 10 s per Fang per kind) |

How activity is recorded (S43-D3, no new dependency):

- `Tracked<T>` wraps the **local** side of the three shared copy paths:
  `proxy_streams` (QUIC; also failover), `secure_copy_client_side`
  (encrypted TCP; also routed and failover), and plain TCP's
  `copy_bidirectional`. Each successful read or write of at least one
  byte stores a timestamp (one relaxed atomic store).
- The timestamp reaches `Tracked` through a tokio task-local set by
  `FangSlot::serve`, so no signature between the accept loop and the copy
  path changed. Outside a Fang session (the receiver, discovery) the
  wrapper is a pass-through.

`FangSlot::serve(session)` runs the session and returns `None` if it was
evicted. The five accept loops now run their session through it. An
evicted session leaves the local socket armed (QUIC, encrypted TCP,
routed, failover already arm at accept; plain TCP arms on eviction), so
the client sees a reset. The QUIC connection and the receiver's session
close as they do for any other failed session.

No receiver quota, window, timeout or wire change. Half-close behaviour
(M1 §5) is unchanged.

## 2. Unit tests (`transport/fang_accept/tests.rs`, 6 new)

| Test | Shows |
|---|---|
| `full_fang_evicts_its_longest_idle_connection` | ceiling 2, threshold 300 ms: the newcomer is admitted, the longest-idle client is reset, the other keeps working |
| `recently_active_connections_are_never_evicted` | activity in either direction keeps a connection; the newcomer is reset instead |
| `a_source_address_is_held_to_its_share` | share 2: the third connection from one source is reset; a released slot returns to the share |
| `slots_and_shares_are_released_when_sessions_end` | a source at its share on a full Fang evicts only its own idle connection, exactly one; afterwards the registry is empty and every permit is back |
| `only_non_loopback_sources_are_shared` | loopback v4, `127.0.0.2`, `::1` and IPv4-mapped loopback are exempt; `10.43.0.2` and its mapped form share one count; production values 60 s / 16 |
| `tracked_records_activity_only_inside_a_fang_session` | reads and writes record activity inside a session; outside one nothing is recorded |

The tests use real time with short thresholds. Run 20 times in a row:
0 failures.

## 3. Lab on the M2 build (one run each)

Lab build `werewolfd` `9649ad08c6f87bb0297c551d3ae262b16e276db119bd2faad3b7feeab9b10ee5`.
Results in `target/stage43-m2/` (`*-20261110T083255Z.json`). M3 repeats
these formally.

**M1 scenarios:**

| Scenario | M1 (`main`) | M2 |
|---|---|---|
| idle:quic | legit after 65 s: **reset** | legit at ceiling reset (connections < 60 s idle), after 65 s: **PASS** |
| idle:tcp | reset | **PASS** |
| idle:tcp-plain | reset | **PASS** |
| source:quic | flood held 64; legit from 2nd source reset at once and after 30 s | flood held **16**; legit **PASS** at once and after 30 s active |
| source:tcp | same | held 16; legit PASS / PASS |
| source:tcp-plain | same | held 16; legit PASS / PASS |

"Legit at ceiling" is still reset in the idle scenario: the 64
connections are seconds old, under the 60 s threshold. That is the
intended rule.

**Stage42 flood scenarios:** unchanged from Stage42 M3. Every Fang keeps
listening and recovers after descriptor exhaustion (`nofile` 256 and
64); QUIC at 64 holds 1.4% CPU, 188 bytes of log; quota scenarios at 63
to 64 echoed, 145 descriptors at most, 17–18 at 5 s and 30 s after
release. "Legit during" for encrypted and plain TCP is still reset: that
flood is active and from loopback, the case M2 does not fix (above).

Daemon alive in every run; the log guard never triggered; host `/tmp`
5%.

## 4. Existing checks

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --workspace --all-targets --all-features -D warnings`:
  PASS.
- `cargo test --workspace --all-features`: **424 passed**, 0 failed
  (418 plus the 6 new tests).
- `scripts/ci-local.sh` full (`target/ci-local/20261110T084306Z`): **13
  of 13 PASS**, including acceptance (all transports, a 50 MiB
  transfer), the routed-Fang gate and the service gates.

## 5. Next

M3: the M1 and Stage42 flood scenarios three times each; because the
copy paths changed, the Stage32 cells Stage41 M3 ran (S2 P0/1/2/5, S3
P0/1, S4 P0/2, S5 P0, S7 P0); and the routed-Fang gate. Every
historical verdict must hold.
