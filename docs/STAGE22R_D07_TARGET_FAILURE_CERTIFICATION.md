# Stage22R-D07: real-WAN target failure certification

Source baseline: `main` at `b911e4146142a81f80291b419e3bc734a870f3e7`, the
build deployed and certified in
[D06](STAGE22R_D06_REPEATED_LOSS_RECOVERY_CERTIFICATION.md). Runs executed
on 2026-10-20 between 14:28 and 14:43 (`+01:00`). This document records
real-WAN evidence from the DarkSide initiator and operator-reported evidence
from the Debian VPS receiver. No production code, protocol, release
artifact, firewall setting or network interface configuration changed here.
Historical local wolves (PIDs 1270 `wolf-a`, 1273 `wolf-b`) and the VPS
echo target (PID 4481) were untouched.

## Purpose and fault method

D07 asks what the application on the initiator sees when the **target**
behind the receiver fails mid-session while both Werewolf daemons stay up:
every failure must reach the application as a failure (never a session that
looks complete), every received byte must be an exact prefix of what was
sent, and a legitimate early half-close by the target must arrive as a clean
end of that direction only. This is the target-side counterpart of the
session-end fixes in
[SESSION_END_SIGNALLING_PROPOSAL.md](SESSION_END_SIGNALLING_PROPOSAL.md).

Disposable test infrastructure outside the repository:

| File | SHA-256 |
|---|---|
| `target.py` (VPS `~/stage22r-d07/`, from DarkSide `/tmp/stage22r-d07/`) | `ca613f259efaf3983e0c50a922e312a8932c10292e1d3cdf86b9580d10b30ddc` |
| `/tmp/stage22r-d07/client.py` | `ae46174f70e0594d6fa906e587ecb2aae409c9918baf62af5068296c94d9adf7` |
| `/tmp/stage22r-d07/run.sh` (driver) | `e4dab466734e934815a8dce36ffdd00678db8af960116583e2677e893860ee7d` |

The target listened on VPS `127.0.0.1:7100` (operator-started, PID 464514)
and forked one child per connection. The client connected to a local Fang,
sent `D07 <mode> <n>`, then 64 MiB of random data while reading concurrently.
With `n` = 8 MiB, the child echoed and then:

| Mode | Target child after echoing 8 MiB |
|---|---|
| `echo` | Echoes everything, closes after the client's EOF (control) |
| `rst` | `SO_LINGER 0` and close: abortive close, RST |
| `kill` | SIGKILLs itself (real crash); its kernel closes the socket with RST if unread data is queued, else FIN |
| `fin` | `shutdown(SHUT_WR)` (clean FIN), keeps reading and discarding until the client's EOF |
| `absent` | Target listener stopped; nothing listens on `127.0.0.1:7100` |

The client records separately how receive and send ended (EOF/complete,
reset, broken pipe, timeout) and whether the received bytes are an exact
prefix of the sent bytes. A reset collected by either thread counts as the
session's reset (D06). On Linux a reset that arrives after the socket has
already received FIN (state `CLOSE_WAIT`) is reported to the next call as
`EPIPE`, not `ECONNRESET`; this matters for `fin` and is why `kill` can be
ambiguous (below).

Pass criteria, fixed before the runs:

| Mode | PASS iff |
|---|---|
| `echo` | 64 MiB echoed exactly, no reset |
| `rst`, `kill` | a reset was seen, the prefix is exact, the session did not look complete |
| `fin` | receive ended with EOF after exactly 8 MiB with an exact prefix; how the send direction ended is recorded, not judged |
| `absent` | a reset was seen and 0 bytes were received |

## Setup

Both nodes ran `werewolfd`
`a71ad688ad120e617c1b851969e584ed08175e9dbce18ea55f8daaf9793fb0c9`.
Paths were direct, with no relay: DarkSide `node-b` at
`tcp://51.83.41.28:8443` for TCP and `quic://51.83.41.28:9560` for QUIC.

| Change | Node | Reverted |
|---|---|---|
| `target allow node-a2 127.0.0.1:7100` | VPS | `target remove`; list back to `127.0.0.1:7000` only; generation 16 before, 18 after |
| Fang profiles `d07-tcp` (local `127.0.0.1:17100`) and `d07-quic` (`127.0.0.1:17101`) to `127.0.0.1:7100` | DarkSide | Both removed; profiles back to `wan-quic`, `wan-tcp` |
| Target process 464514 | VPS | Stopped by exact PID for `absent` |

Profile and Pack-address switches happened only between series
(`fang deactivate`, `pack set-address`, `fang activate`).

## Results

| Series | Runs | PASS | Excluded | Received | Receive / send end |
|---|---|---|---|---|---|
| TCP `echo` | 3 | 3 | 0 | 64 MiB, exact | EOF / complete |
| TCP `rst` | 5 | 5 | 0 | 3,163,416–8,388,608, exact prefix | reset / EPIPE, or EOF / reset |
| TCP `kill` | 5 | 4 | 1 (`kill-5`) | 3,759,824–8,388,608, exact prefix | as `rst`; `kill-5`: EOF / EPIPE |
| TCP `fin` | 5 | 5 | 0 | exactly 8,388,608, exact | EOF / EPIPE |
| QUIC `echo` | 3 | 3 | 0 | 64 MiB, exact | EOF / complete |
| QUIC `rst` | 5 | 5 | 0 | 8,340,770–8,388,608, exact prefix | reset / EPIPE, or EOF / reset |
| QUIC `kill` | 5 | 4 | 1 (`kill-4`) | 8,348,748–8,388,608, exact prefix | as `rst`; `kill-4`: EOF / EPIPE |
| QUIC `fin` | 5 | 5 | 0 | exactly 8,388,608, exact | EOF / complete |
| QUIC `absent` | 3 | 3 | 0 | 0 | reset within 0.26–0.48 s |
| TCP `absent` | 3 | 3 | 0 | 0 | reset within 0.45–0.46 s |
| **Total** | **42** | **40** | **2** | | |

No run produced a byte that was not an exact prefix of the sent data, and no
failed session looked complete. Durations: `rst`/`kill` 3.1–8.8 s, TCP `fin`
4.1–6.9 s, QUIC `fin` 53.4–54.7 s (the full 64 MiB upload), `echo` 28–32 s
(TCP) and 57–64 s (QUIC).

**`fin` differs by transport, as designed.** Over QUIC the two stream
directions are independent: the application received EOF after exactly
8 MiB and its remaining 56 MiB upload completed to the target, which kept
reading. Over encrypted TCP the initiator ends the whole session once the
remote direction has finished (owner decision 5 of the session-end
proposal), so the unfinished upload ends with a reset, reported as `EPIPE`
because the socket had already received FIN. In all five TCP `fin` runs
that reset arrived after the application had read all 8 MiB, so it cost no
downloaded byte.

**The two excluded `kill` runs** (TCP `kill-5`, target child 464568; QUIC
`kill-4`, child 464605) received EOF after exactly 8 MiB, then `EPIPE` on
send and no `ECONNRESET`: the `fin` pattern. That is the expected outcome
if the dying child's kernel had no unread data queued and so sent FIN, which
Werewolf then correctly propagated; the later upload met the dead socket
and was reset. The target log shows each child had read 688–62,387 bytes
more than it echoed, but data still queued in the kernel at the moment of
death is not observable without a packet capture, so FIN versus RST is not
proven and the runs are excluded rather than counted as PASS. Either way
the application was not shown a complete session: it received 8 MiB of
64 MiB and its send failed.

## Resource and state stability

DarkSide daemon PID 635922, SHA-256 `a71ad688…` in all 50 snapshots;
every status `READY`, `storage_degraded=false`, `inbound_authority.active=0`:

| Series | Generation | Descriptors | RSS (kB) |
|---|---|---|---|
| TCP | 51 | 17 | 9,764 flat |
| QUIC | 54 | 18–19 | 12,596 → 12,748 |
| QUIC `absent` | 54 | 18 | 16,096 flat |
| TCP `absent` | 57 | 17 | 16,096 flat |
| After removing the profiles | 60 | 16 | 16,100 |

Descriptors and threads did not grow. RSS rose in two steps outside the
sessions (on the switch to QUIC, and in the idle 1.5 minutes before the
`absent` series) and stayed flat across sessions within each series.

Follow-up the same day (15:06–15:22), on the same daemon, idle between
steps: of 16,100 kB RSS, 7,580 kB was file-backed (4,672 kB of
`werewolfd`'s own text, paged in as code paths run for the first time and
bounded by the 8.9 MB binary) and 8,520 kB anonymous, almost all in four
64 MB-aligned regions, one glibc malloc arena per tokio worker
(3,260 / 2,076 / 2,064 / 360 kB; the main `[heap]` held 116 kB). Repeated
8 MiB echo sessions through the direct WAN path to the VPS echo target,
in batches of four sequential and four concurrent sessions, then gave:

| Load | Sessions | RSS change | Descriptors, threads |
|---|---|---|---|
| Encrypted TCP | 48 | +4 kB | flat (17, 5) |
| QUIC Fang activation | — | +1,112 kB (endpoint) | +1 descriptor |
| QUIC | 48 | +388 kB (two steps, both in concurrent batches) | flat (18, 5) |
| QUIC, continued | 96 | +152 kB (two steps, concurrent batches 8 and 9); none in the first 64 or last 24 | flat (18, 5) |

Growth does not scale with sessions; it appears only when concurrency
reaches a new high and then stops, which matches allocator arenas keeping
their high-water mark and first-time code paging. No per-session leak was
found. The idle step before the `absent` series was not reproduced and
remains unattributed.

VPS receiver after the series: MainPID 463493, `NRestarts=0`, lifecycle
`READY`, `inbound_authority.active=0`, healthy, not locked, not degraded,
generation 18; 16 descriptors (16 before), 3 threads, RSS 9,620 kB
(9,288 before). Echo target 4481 still running since 2026-09-23.

## Certification conclusion

On both transports, every target failure that could be attributed —
abortive close, crash with RST, and a missing target — reached the
application as a reset with an exact prefix and never as a complete
session; a clean target half-close arrived as a clean end of the download
after exactly the bytes sent, with the upload continuing on QUIC and reset
on encrypted TCP per decision 5. Neither daemon restarted, leaked
descriptors or retained inbound authority. **PASS** for 40 of 42 runs;
2 `kill` runs excluded as unattributable (FIN versus RST at crash).

Stage22R-D07: **CERTIFIED** (QUIC and encrypted TCP), with the exclusion
above.

Not covered here: a target that stalls without closing (no Werewolf-level
idle timeout applies to a live target), and target failure under packet
loss.
