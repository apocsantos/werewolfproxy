# Stage22R-D06: real-WAN repeated loss and recovery certification

Source baseline: `main` at `b911e4146142a81f80291b419e3bc734a870f3e7`.
Runs executed on 2026-10-20 (times `+01:00`). This document records
real-WAN evidence from the DarkSide initiator and operator-reported evidence
from the Debian VPS receiver. No production code, protocol, release
artifact, firewall setting or network interface configuration changed here.
Historical local wolves (PIDs 1270 `wolf-a`, 1273 `wolf-b`) were untouched.

## Purpose and fault method

D05A removed the path once; D05B paused it once. D06 asks whether **repeated**
outages on daemons that are never restarted leave either side degraded:
sessions that should survive must keep exact integrity on every outage, and
sessions that should die must fail visibly every time, with a fresh session
working after each loss and no growth in descriptors, threads or authority.

Two parts per transport:

- **Part A, repeated temporary blackholes on one session.** One relay
  process, one 1 GiB full-duplex echo transfer on **one** TCP connection to
  the local Fang, eight 10 s pauses: the first after 4 MiB forwarded, then
  one every further 96 MiB (both directions summed), so all eight fall in
  the first ~700 MB of the ~2.2–2.7 GB the relay carried. Valid outcome:
  the same session survives every pause with exact 1 GiB integrity.
- **Part B, repeated hard path loss.** Ten cycles. Each cycle starts a
  fresh relay, sends an exact-echo precheck marker, starts a 256 MiB
  transfer, and the relay SIGKILLs itself after 16 MiB forwarded. The
  transfer must fail visibly (never hang, never succeed), then a recovery
  relay (`trip-kind none`) is started and a recovery marker must echo
  exactly on a fresh session.

Disposable test infrastructure outside the repository:

| File | SHA-256 |
|---|---|
| `/tmp/stage22r-d06/chaos_proxy_cycles.py` (D05B relay plus `--cycles`/`--cycle-every`) | `a4dd09d7a08cd26a26ffb1a61152aebe2c33ed740b778c5d6f119c1d4078a9eb` |
| `/tmp/stage22r_chaos_proxy.py` (unchanged D05B relay, for reference) | `5e99709c0d96c5125bde95347a0bbf78b4249e54311a35203844990f7639e866` |
| `/tmp/stage22r_duplex_client.py` (unchanged D05B client) | `eb71c0aa4fd26419def19fe2e8f42bca7acef921b74b28fe866b68d01fe9e196` |
| `/tmp/stage22r-d06/run.sh` (driver, both parts) | `0fa834bdfeda58efbb2875d0bb4ab5253135a5ec013375a6a764b63551fe8b4a` |

The cycle relay differs from the D05B relay only in re-arming the pause
trip: trip *n* fires at `threshold + (n-1) × cycle-every` forwarded bytes,
never while a pause is in progress, at most `--cycles` times. Pause
semantics are those of D05B: QUIC datagrams are **discarded** in both
directions during a pause; encrypted TCP is **stalled** (no byte read or
written, none dropped), so TCP retransmission backoff is not exercised.

The driver snapshotted the DarkSide daemon before and after every run and,
in Part B, after every loss and every recovery: PID, binary SHA-256, open
descriptors, threads, RSS, `status` and `fang active`.

## Topology and nodes

Both nodes ran the same build of `b911e41` (`cargo build --locked
--release`), copied to the VPS by the operator and checked on both hosts:

| Binary | SHA-256 |
|---|---|
| `werewolfd` | `a71ad688ad120e617c1b851969e584ed08175e9dbce18ea55f8daaf9793fb0c9` |
| `werewolfctl` | `360ea359a6c548c6db987eb20260ed6265afbce1d2ce7a790cf1dcc66a170e32` |

DarkSide initiator: `werewolfd` PID 635922, installed at
`~/.local/lib/werewolf-b911e41/bin/`, Den `~/.werewolf-stage22r`, started
with `--listen 127.0.0.1:28443 --quic-listen 127.0.0.1:29560`. On start it
restored its active `wan-tcp` profile by itself. The same PID and SHA-256
were recorded in every snapshot of every run.

VPS receiver: upgraded from `ea9ab163…` / `c9c5e5cf…` to the build above
following `packaging/INSTALL.md` (service stopped, Den and previous binaries
backed up to `/root/werewolf-backup/20261020-b911e41/`, binaries replaced,
service started). Afterwards `werewolfd.service` MainPID 463493,
`NRestarts=0`, lifecycle `READY`, generation 16, `storage_degraded=false`,
packmates 1; `doctor` `OK_OR_WARNING` with all checks `OK`. The unit and its
`10-wan-bind.conf` drop-in (`0.0.0.0:8443`, `0.0.0.0:9560`) were unchanged.
Target `python3 /tmp/werewolf-echo-trace.py` on `127.0.0.1:7000`, PID 4481,
running since 2026-09-23 and not restarted.

Pack and Fang changes happened only between runs, never during one:

| Series | Pack `node-b` | Relay | Fang | DarkSide generation during runs |
|---|---|---|---|---|
| TCP A | `tcp://127.0.0.1:18443` | TCP `127.0.0.1:18443` → `51.83.41.28:8443` | `wan-tcp`, local `127.0.0.1:17000` | 39 |
| QUIC A, QUIC B | `quic://127.0.0.1:19560` | UDP `127.0.0.1:19560` → `51.83.41.28:9560` | `wan-quic`, local `127.0.0.1:17001` | 42 |
| TCP B | `tcp://127.0.0.1:18443` | TCP `127.0.0.1:18443` → `51.83.41.28:8443` | `wan-tcp`, local `127.0.0.1:17000` | 45 |

Each switch was `fang deactivate`, `pack set-address`, `fang activate`.

## Part A: repeated temporary blackholes

| | Encrypted TCP | QUIC |
|---|---|---|
| Relay PID | 637163 | 647506 |
| Pauses | 8 × 10.0003–10.0009 s, 12:31:23.752–12:34:46.545 | 8 × 10.0003–10.0008 s, 12:42:04.220–12:48:24.071 |
| Dropped c2u / u2c | 0 / 0 (stall) | 65,251 / 507,985 bytes |
| Same session | one relay TCP connection (conn 2) for the whole transfer | one UDP client (`127.0.0.1:43443`) for the whole run |
| TX / RX bytes | 1,073,741,824 / 1,073,741,824 | 1,073,741,824 / 1,073,741,824 |
| SHA-256 (TX = RX) | `cc18f1d34bf5b9c9fa78ec66996e13631ef542bcd3910311cac899db8f9b0402` | `6c5b881ef0299fd25e3529c2d75b4b3076a5d69d3b9e5d2e66e25e7ad94a6e17` |
| Client duration | 616.686 s | 993.394 s |
| Longest RX gap | 9.984 s | 15.295 s |
| Relay carried c2u / u2c | 1,352,840,160 / 1,353,127,338 | 1,105,805,219 / 1,109,497,624 |

Both transfers: precheck `MATCH=True`, `SIZE_MATCH=True`, `HASH_MATCH=True`,
no errors, clean EOF, exit 0. Outcome A on every pause. The QUIC 15.3 s
longest gap means delivery resumed up to ~5 s after a pause ended, the same
loss-probe backoff D05B observed after its 20 s pause.

VPS target lines for these transfers: see [VPS target](#vps-target).

## Part B: repeated hard path loss

| | Encrypted TCP | QUIC |
|---|---|---|
| Cycles | 10, 13:14:16–13:15:02 | 10, 13:06:46–13:13:19 |
| Prechecks | 10/10 `MATCH=True` | 10/10 `MATCH=True` |
| Cut transfers | 10/10 `FAIL`, exit 1, `SIZE_MATCH=False` | 10/10 `FAIL`, exit 1, `SIZE_MATCH=False` |
| Relay SIGKILL → client sees failure | 0.002–0.003 s | 30.001–30.115 s |
| RX bytes before the cut | 2,954,584–5,878,072 | 8,082,088–8,103,035 |
| Recovery markers | 10/10 `MATCH=True` | 10/10 `MATCH=True` |

Over QUIC every cut session ended at `QUIC_IDLE_TIMEOUT = 30 s`
(`crates/werewolfd/src/transport/quic.rs`), as in D05A/D05B; over encrypted
TCP the relay's sockets close with the process and the session ended at
once. No transfer hung: the client's 90 s idle limit was never reached.

Every cut transfer reached the application as a **TCP reset**. Each cycle
shows exactly one `ConnectionResetError` on the client socket, on whichever
of the client's two threads made the next call:

| | reset seen by receive (send then `BrokenPipeError`) | reset seen by send (receive then EOF) |
|---|---|---|
| Encrypted TCP | 8 cycles | 2 cycles (c03, c09) |
| QUIC | 6 cycles | 4 cycles (c03, c04, c06, c09) |

Linux reports a pending socket error once, to the first call that collects
it; after that the socket is closed, so the other thread's `recv` returns 0
and its `send` returns `EPIPE`. The receive EOF in the second column is
therefore a property of the test client's two-thread design, not a clean
FIN from the Fang: in each such cycle the same client's send failed with
`ConnectionResetError`. An application must treat any reset on the
connection as the end of a failed session.

## Resource and state stability

DarkSide daemon, PID 635922 and SHA-256 `a71ad688…` in every snapshot:

| Run | Snapshots | Descriptors | Threads | RSS (kB) | Status in every snapshot |
|---|---|---|---|---|---|
| TCP A | 3 | 17 → 17 | 5 → 5 | 8,152 → 8,572 | `READY`, gen 39, `active=0`, 1 Fang `Active` |
| QUIC A | 3 | 18 → 18 | 6 → 5 | 8,600 → 9,476 | `READY`, gen 42, `active=0`, 1 Fang `Active` |
| QUIC B | 22 | 18 after each loss, 19 after each recovery | 5 | 9,476 → 9,652 | `READY`, gen 42, `active=0`, 1 Fang `Active` |
| TCP B | 22 | 17 | 6 → 5 | 9,536 → 9,560 | `READY`, gen 45, `active=0`, 1 Fang `Active` |

`storage_degraded=false` throughout. In QUIC B, RSS grew by 12–44 kB in each
of the first six cycles and not at all in the last four (9,632–9,652 kB),
which is consistent with allocator warm-up rather than a per-session leak.

VPS receiver, MainPID 463493 throughout:

| Reading | Descriptors | Threads | RSS (kB) | `NRestarts` | Status |
|---|---|---|---|---|---|
| During TCP A (one session open) | 18 | 3 | 8,184 | — | — |
| Idle, after Part A on both transports | 16 | 3 | 9,244 | 0 | `READY`, gen 16, `active=0`, healthy |
| Idle, after Part B on both transports | 16 | 3 | 9,288 | 0 | `READY`, gen 16, `active=0`, healthy, not locked, not degraded |

## VPS target

Operator-reported lines from the echo target (PID 4481, read through
`/proc/4481/fd/1`), one connection per transfer, each ending in EOF:

| Transfer | Target RX / TX | Target elapsed | Client duration |
|---|---|---|---|
| TCP A | 1,073,741,824 / 1,073,741,824 | 616.337 s | 616.686 s |
| QUIC A | 1,073,741,824 / 1,073,741,824 | 993.277 s | 993.394 s |

A third matching line (`elapsed=995.534`) precedes both in the log and
belongs to an earlier transfer, so it is excluded. Prechecks, recoveries
and the cut Part B transfers were not requested from the target log.

## Certification conclusion

Across 16 temporary blackholes and 20 hard path losses on daemons that were
never restarted, no outage produced a false success, a hang, a daemon
restart, storage degradation, retained inbound authority or descriptor or
thread growth on either node.

- **Temporary blackholes:** the same session survived eight 10 s blackholes
  with exact 1 GiB integrity on both QUIC (datagrams discarded) and
  encrypted TCP (stall). **PASS.**
- **Hard path loss:** all 20 cut sessions failed visibly with a TCP reset to
  the application (QUIC at the 30 s idle timeout, encrypted TCP at once),
  and all 20 fresh sessions afterwards echoed exactly, with no restart and
  no Pack or Fang change. **PASS.**

Stage22R-D06: **CERTIFIED** (QUIC and encrypted TCP).

### Relation to the D05B follow-up

D05B lists as a follow-up that "when a QUIC session dies, the local
application sees receive EOF before the send error". Its failed runs show
receive EOF with `BrokenPipeError` on send and no `ConnectionResetError`
on either call, which is consistent with the Fang ending those sessions
with a FIN rather than a reset, so truncation could look like completion. That build predates the session-end fixes of
[SESSION_END_SIGNALLING_PROPOSAL.md](SESSION_END_SIGNALLING_PROPOSAL.md).
On `b911e41` every one of the 20 cut sessions delivered a reset, collected
by exactly one client call, so the D05B observation is resolved on this
build; its record is left unchanged as evidence of the older behaviour.

Not covered here: the VPS target failing mid-session (D07), and TCP packet
loss with retransmission backoff (needs firewall changes outside this
method).
