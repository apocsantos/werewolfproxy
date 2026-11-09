# Stage42 M1 — Fang connection flood, reproduced

**M1: CONFIRMED.** Every survey hypothesis that M1 could test
reproduced on `main`:

1. **Self-denial of service.** A flood through a QUIC or encrypted-TCP
   Fang takes all 64 of the peer's session slots. A legitimate session
   started during the flood **fails**.
2. **Encrypted-TCP and plain-TCP Fangs die on descriptor exhaustion.**
   Their accept loop ends on `EMFILE`. The Fang stops listening, drops
   out of `fang.list`, and does **not** come back once descriptors free
   up, while the daemon keeps running.
3. **The QUIC Fang spins on descriptor exhaustion.** Its loop retries
   `accept` with no delay and logs every failure. That held **103% CPU**
   and wrote about **6.5 MB/s** of log (§5).
4. **Plain-TCP Fangs are unbounded.** 500 connections gave 422 echoes and
   **1017** descriptors. No remote quota applies.

No production source changed. The tool is `tools/stage42/fang_flood.py`.

## 1. Baseline

- `main` `610eb0e`, lab debug build `werewolfd`
  `3a96a23f2c9c2cc615468121d26fde284876d11edb1c89e747e1a4e416444f34`.
- DarkSide, Linux 6.12.115, rustc 1.96.0.
- Owner decisions S42-D0–D5 as recommended.

## 2. Method

`fang_flood.py` reuses the acceptance lab (`tests/integration/run.py`):

- fresh Dens and dynamic loopback ports for each scenario;
- daemon A is the receiver, with its Target Policy allowing only the
  lab's echo target;
- daemon B opens one Fang of the scenario's kind toward A, with the echo
  target as remote;
- only the lab's own process groups are stopped, and no live wolf is
  touched.

Scenarios:

- **`fd:<kind>`:** B runs under `prlimit --nofile=N`. The client opens up
  to 600 connections to the Fang and holds them. The tool records B's
  descriptors, its CPU over 5 s, and whether the Fang still accepts. It
  then releases the flood and checks again, plus a new legitimate 1 KiB
  echo.
- **`quota:<kind>`:** no limit. 500 connections each send 16 bytes and
  wait up to 20 s for the echo. The tool records how many are echoed, a
  legitimate echo during the flood, and the same checks after release.

Routed and failover Fangs were not run. Their accept loops use the same
`listener.accept().await?` as the encrypted-TCP Fang (`relay.rs:148`,
`failover.rs:173`; CONFIRMED from code). The survey's fix covers all
five.

## 3. Quota scenarios (no descriptor limit)

Results `target/stage42-m1/m1-20261109T192521Z.json`.

| Fang | Connections | Echoed | Refused (closed) | B descriptors (idle → flooded → released) | Legit echo during | Legit echo after |
|---|---|---|---|---|---|---|
| QUIC | 500 | **64** | 436 | 20 → 82 → 18 | **FAIL** | PASS |
| encrypted TCP | 500 | **64** | 436 | 21 → 145 → 17 | **FAIL** | PASS |
| plain TCP | 500 | 422 (in 20 s) | 0 | 19 → **1017** → 173 | PASS | PASS |

- **QUIC and encrypted TCP:** exactly the receiver's `PEER_SESSIONS` (64)
  were served. The rest were refused remotely after B had already paid
  for the connection. The operator's own session then fails, which is the
  self-denial of service.
- **Plain TCP:** no quota. Descriptors grow with the flood, two per
  session.
- **UNEXPLAINED:** 173 descriptors were still open 5 s after the
  plain-TCP flood was released. That could be slow teardown of
  partly-echoed sessions or a leak. M3 checks it with a longer settle
  time.

## 4. Descriptor-exhaustion scenarios

| Fang | `nofile` | Result | After release |
|---|---|---|---|
| QUIC | 256 | did not exhaust: 600 connections, 83 descriptors; refused sessions close quickly | normal |
| encrypted TCP | 256 | Fang **stopped listening** after 158 connections; the other 442 were refused by the kernel | still not listening, gone from `fang.list`, legit echo FAIL; daemon alive |
| plain TCP | 256 | Fang **stopped listening** after 154 connections, at 256 descriptors | same as encrypted TCP |
| QUIC | **64** | **spin:** 64/64 descriptors, **103% CPU**; log 54 MB when the tool's guard stopped it | (stopped by the guard) |
| encrypted TCP | 64 | Fang stopped listening after 50 connections; one log line | not listening, gone from `fang.list`, legit FAIL |

Results: `target/stage42-m1/m1-20261109T192521Z.json` (256) and
`m1-nofile64-*.json` (64).

The surviving error is `accept failed: Too many open files (os error
24)` (QUIC, repeated). The encrypted-TCP and plain-TCP Fangs exit
through `?` with one line.

## 5. Incident during M1: the QUIC spin filled `/tmp`

The first QUIC run at `nofile=64` had no log guard. The spinning Fang
logged about **80 million** `accept failed` lines (3.95 GB) in about 10
minutes, into the lab's own directory on `/tmp`, a 3.9 GB tmpfs on
DarkSide. That **filled `/tmp`** from about 19:28 to 19:38 UTC on
2026-11-09.

- The lab daemon then received SIGTERM and stopped. The lab directory
  was removed after excerpts were saved to
  `target/stage42-m1/nofile64-logs/`, which `/tmp` recovered at once (5%
  used).
- The live wolves' processes and sockets were still present afterwards.
  They were not inspected further or touched. Other `/tmp` users on the
  host may have seen write failures in that window.
- The tool now has a guard. Every wait checks the lab's logs. Above
  50 MB it records B's descriptors and CPU, then stops the lab daemons
  at once. The flood also stops after 10 consecutive connect timeouts.
  The guarded rerun stopped at 54 MB.

This is itself evidence for the survey: in production, under systemd,
the same spin would go to the journal (rate-limited) while holding a
core.

## 6. Conclusion

All four hypotheses the survey listed for M1 are CONFIRMED. M2 can
proceed with the approved design:

- a per-Fang ceiling of 64 and a global ceiling of 1024;
- a connection over either ceiling is accepted and reset at once;
- on an accept error, every Fang kind backs off and keeps listening,
  and logs at a bounded rate.

The last point follows from §5. It is part of the approved D4 "accept
error back-off" and is noted here so M2 implements it.
