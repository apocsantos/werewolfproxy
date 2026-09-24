# Stage22R-D01: real-WAN receiver-restart certification

Source baseline: `main` at `cfbd52a71ac957fc8ad1ac77509cf13ac15c6758`.
This document records operator-reported real-WAN evidence. No production
code, protocol, service configuration, or release artifact changed here.

## Purpose and topology

D01 tests whether restarting the remote receiver daemon during an active,
sustained transfer fails closed and whether fresh authenticated sessions
recover afterward. The DarkSide initiator connected over the real Internet
to a Debian VPS receiver, which forwarded to target `127.0.0.1:7000`. The
receiver's systemd unit was `werewolfd.service`.

## QUIC receiver restart

An active 1 GiB transfer was confirmed by a new target connection. Before
restart, receiver inbound authority reported active = 1, epoch = 1,
locked = false, and healthy = true. The receiver was restarted with
`systemctl restart werewolfd.service`.

After restart, the daemon was active/running. Inbound authority reported
active = 0, epoch = 1, healthy = true, locked = false, and lifecycle = `READY`.
Packmates = 1, protected-state generation = 16, and storage_degraded = false.

The interrupted transfer produced the following result:

| Field | Observed |
|---|---|
| Expected bytes | 1,073,741,824 |
| TX bytes | 185,925,632 |
| RX bytes | 183,990,820 |
| TX SHA-256 | `fa9d4db92a6e9ceaef471fd313476d3b1823d826616cab5989d1d9d828828b10` |
| RX SHA-256 | `50439d547cd8870db5fa6645b70fd66370885147dd625535e93023d7fd488a4f` |
| HASH_MATCH / SIZE_MATCH | `False` / `False` |
| TX_ERROR / RX_ERROR | `BrokenPipeError [Errno 32]` / `None` |
| Transfer result / process exit | `FAIL` / `1` |

The independent VPS target observed RX = 183,990,820 and TX = 183,990,820
bytes, followed by clean EOF. A fresh QUIC session after receiver restart
echoed `STAGE22R-D01-QUIC-RECOVERY` exactly (`MATCH=True`). The interrupted
connection did not report success or accept incomplete data as a complete
transfer; persistent Pack/protected state remained usable and a fresh QUIC
session recovered. **D01 QUIC receiver-restart gate: PASS.**

## Encrypted TCP receiver restart

The Pack endpoint changed from QUIC to TCP. LC01 invalidated the affected
active Fang (`closed_fangs=1`). The operator explicitly activated `wan-tcp`:
local listener `127.0.0.1:17000`, remote target `127.0.0.1:7000`, peer
`51.83.41.28:8443`, transport `tcp`. The precheck echoed
`STAGE22R-D01-TCP-PRECHECK` exactly (`MATCH=True`).

An active 1 GiB transfer was confirmed by a target `CONNECT` and receiver
`inbound_authority.active=1`. The receiver daemon was restarted during that
transfer. After restart, the daemon was active/running. Inbound authority
reported active = 0, epoch = 1, healthy = true, locked = false, and
lifecycle = `READY`. Packmates = 1, protected-state generation = 16, and
storage_degraded = false.

The interrupted transfer produced the following result:

| Field | Observed |
|---|---|
| Expected bytes | 1,073,741,824 |
| TX bytes | 109,510,656 |
| RX bytes | 105,251,592 |
| TX SHA-256 | `18f0168e4615d98995a7c0bd9f7594b08033ef95912af18497dbac5483a5dfd2` |
| RX SHA-256 | `de1311409c6e97b755e1f78fa49b061adffa05558cb56d1f23b243551e4bfa0c` |
| HASH_MATCH / SIZE_MATCH | `False` / `False` |
| TX_ERROR / RX_ERROR | `ConnectionResetError [Errno 104]` / `None` |
| Transfer result / process exit | `FAIL` / `1` |

The independent VPS target observed RX = 105,469,992 and TX = 105,469,992
bytes, followed by clean EOF. A fresh encrypted TCP session after receiver
restart echoed `STAGE22R-D01-TCP-RECOVERY` exactly (`MATCH=True`). The
interrupted connection surfaced failure, did not falsely claim integrity,
and a fresh TCP session recovered with persistent receiver state intact.
**D01 encrypted-TCP receiver-restart gate: PASS.**

## Certification conclusion

Both secure transports failed closed when the receiver restarted mid-stream:
the interrupted 1 GiB transfers failed visibly, with neither size nor hash
equality, rather than reporting silent success. The restarted receiver was
healthy and `READY`, retained its Pack/protected state, and served fresh
authenticated QUIC and encrypted-TCP sessions. D01 receiver-restart behavior
is certified for both transports on the tested real-WAN topology. This does
not claim that an interrupted transfer resumes or that its unsent bytes are
delivered.
