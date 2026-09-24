# Stage22R-D03: real-WAN initiator hard-crash certification

Source baseline: `main` at `e4e782a2e709eb5d5eaeb63ce30bb37cfaa16ac2`.
This document records operator-reported real-WAN evidence. No production
code, protocol, service configuration, or release artifact changed here.

## Purpose and isolation

D03 tests fail-closed behavior and persistent recovery when the DarkSide
Stage22R initiator is killed with SIGKILL during an active transfer to a
Debian VPS receiver. The receiver forwarded to target `127.0.0.1:7000`.
Only secure QUIC and encrypted TCP were tested. The Stage22R HC02 binary was
`/home/sleepdeprivedloon/.local/lib/werewolf-stage22r-hc02/bin/werewolfd`,
SHA-256 `8dad5819d99fac16b5519092e8f38da0d3aa8d3c681aa024925706d8c43e13fc`.
The historical local daemons, PID 1225 (`wolf-a`) and PID 1226 (`wolf-b`),
were preserved and untouched throughout.

## Excluded attempts

Two earlier attempts are **not D03 certification evidence**:

1. A PID matcher aborted, leaving a stale PID for the attempted SIGKILL.
   The kill returned `No such process`; no daemon was killed.
2. A later SIGKILL targeted the correct daemon, but a 1 GiB QUIC transfer
   had already completed successfully. That run supplied another QUIC
   integrity pass, not a mid-transfer hard-crash result.

Only the confirmed live-transfer SIGKILL runs below certify D03.

## QUIC hard crash

The daemon had been restarted as PID 263133; its exact HC02 SHA-256 was
confirmed. The active Fang was local `127.0.0.1:17001`, peer `node-b`,
remote target `127.0.0.1:7000`, transport `quic`, state `Active`. The
precheck echoed `STAGE22R-D03-QUIC-HARDKILL-PRECHECK` exactly (`MATCH=True`).
New unique test files were `/tmp/stage22r_d03_quic_hardkill.log` and
`/tmp/stage22r_d03_quic_hardkill.exit`.

Immediately before SIGKILL, the stress process was confirmed active, the
exit file was absent, and the independent VPS target showed `CONNECT` with
no corresponding EOF. The Stage22R PID was resolved again as 263133 and
the exact HC02 SHA-256 reconfirmed. The result was `PASS: STAGE22R
HARD-KILLED MID-TRANSFER`. Historical wolves 1225 and 1226 remained alive.

| Client field | Observed |
|---|---|
| Expected bytes | 1,073,741,824 |
| TX bytes | 140,967,936 |
| RX bytes | 139,084,280 |
| TX SHA-256 | `0e5056d26918a87ae212c7f6a0befbb95ae3ca04fb11a27060f58891c65d0429` |
| RX SHA-256 | `952a2d4372b55094478bfa4d5cb2f2e600fc1c97ba13e716cf31b35b27d6d2b0` |
| HASH_MATCH / SIZE_MATCH | `False` / `False` |
| TX_ERROR / RX_ERROR | `BrokenPipeError [Errno 32]` / `ConnectionResetError [Errno 104]` |
| Transfer result / process exit | `FAIL` / `1` |

The independent VPS target recorded connection port 46356, RX = 139,165,743
and TX = 139,165,743 bytes, elapsed = 140.259126 s, and EOF. The initiator
restarted as PID 263427 with the same exact HC02 SHA-256. Without manual
Fang activation, it reported lifecycle = `READY`, storage_degraded = false,
protected-state generation = 27, and active_fangs = 1. The automatically
restored Fang was local `127.0.0.1:17001`, peer `node-b`, remote target
`127.0.0.1:7000`, transport `quic`, state `Active`. A fresh session echoed
`STAGE22R-D03-QUIC-RECOVERY` exactly (`MATCH=True`).

**D03 QUIC hard-crash gate: PASS.**

## Encrypted TCP hard crash

The Pack endpoint changed from QUIC to TCP. LC01 invalidated the active
Fang (`closed_fangs=1`), and the active Fang list became empty. The operator
explicitly activated `wan-tcp`: Fang ID `fang_d71cf0b8da85815d`, local
`127.0.0.1:17000`, peer `node-b`, peer address `51.83.41.28:8443`, remote
target `127.0.0.1:7000`, transport `tcp`, state `Active`. The precheck echoed
`STAGE22R-D03-TCP-HARDKILL-PRECHECK` exactly (`MATCH=True`). New unique test
files were `/tmp/stage22r_d03_tcp_hardkill.log` and
`/tmp/stage22r_d03_tcp_hardkill.exit`.

Immediately before SIGKILL, stress process PID 263706 was confirmed active,
the exit file was absent, and the transfer was explicitly confirmed still
active. The Stage22R daemon PID was resolved again as 263427 and its exact
HC02 SHA-256 reconfirmed. The result was `PASS: STAGE22R HARD-KILLED
MID-TRANSFER`. Historical wolves 1225 and 1226 remained alive.

| Client field | Observed |
|---|---|
| Expected bytes | 1,073,741,824 |
| TX bytes | 103,743,488 |
| RX bytes | 99,891,504 |
| TX SHA-256 | `dd4ea27c525f774c3a663e811c2ba978a25ed162a085fb26c6204b703070923b` |
| RX SHA-256 | `285e94bb22b95810af69778724ccf81698241936347f6e80f94f2e0cc2a71444` |
| HASH_MATCH / SIZE_MATCH | `False` / `False` |
| TX_ERROR / RX_ERROR | `BrokenPipeError [Errno 32]` / `ConnectionResetError [Errno 104]` |
| Transfer result / process exit | `FAIL` / `1` |

The independent VPS target recorded connection port 35638, RX = 100,041,304
and TX = 100,041,304 bytes, elapsed = 83.889558 s, and EOF. The initiator
restarted as PID 263850 with the same exact HC02 SHA-256. Without manual
Fang activation, it reported lifecycle = `READY`, storage_degraded = false,
protected-state generation = 29, and active_fangs = 1. The automatically
restored Fang had ID `fang_33e199bd43064a7a`, local `127.0.0.1:17000`, peer
`node-b`, remote target `127.0.0.1:7000`, transport `tcp`, state `Active`.
A fresh session echoed `STAGE22R-D03-TCP-RECOVERY` exactly (`MATCH=True`).

**D03 encrypted-TCP hard-crash gate: PASS.**

## Certification conclusion

Both transfers were proven live before SIGKILL, and the intended Stage22R
daemon—not either historical wolf—was killed. Abrupt process death surfaced
transport errors to the local application; neither incomplete 1 GiB
transfer falsely reported success or intact data. The remote target
observed partial data and connection EOF in both cases. Protected state
remained healthy, persistent active-Fang intent survived, listeners
automatically returned after restart without manual `fang activate`, and
fresh authenticated sessions succeeded. QUIC and encrypted-TCP hard-crash
behavior: **PASS**. Stage22R-D03: **CERTIFIED** on the tested real-WAN path.
