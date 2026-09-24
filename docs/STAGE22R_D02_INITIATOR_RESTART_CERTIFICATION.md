# Stage22R-D02: real-WAN initiator-restart certification

Source baseline: `main` at `922ced82056e5f2a2eed1ee156b29dc58a6dfb1c`.
This document records operator-reported real-WAN evidence. No production
code, protocol, service configuration, or release artifact changed here.

## Purpose and isolation

D02 tests fail-closed behavior, persistent active-Fang recovery, and fresh
authenticated-session recovery when the initiating Stage22R daemon receives
SIGTERM during an active real-WAN transfer. The Stage22R HC02 binary was
`~/.local/lib/werewolf-stage22r-hc02/bin/werewolfd`, SHA-256
`8dad5819d99fac16b5519092e8f38da0d3aa8d3c681aa024925706d8c43e13fc`.
The historical local daemons, PID 1225 (`wolf-a`) and PID 1226 (`wolf-b`),
were preserved and untouched throughout both cases.

## Encrypted TCP initiator restart

The precheck echoed `STAGE22R-D02-TCP-PRECHECK` exactly (`MATCH=True`). The
initiator was PID 204483, and its exact HC02 binary SHA-256 was confirmed.
SIGTERM interrupted an active 1 GiB transfer.

| Field | Observed |
|---|---|
| Expected bytes | 1,073,741,824 |
| TX bytes | 232,325,120 |
| RX bytes | 228,010,296 |
| TX SHA-256 | `eb93016682781202d2231bb58b67deaf5f46fab308e5ee178b5d38038d588325` |
| RX SHA-256 | `b4d0fabd29650956d83219e903c6b4159647e5b8ea7fbf427433a18f5673e21e` |
| HASH_MATCH / SIZE_MATCH | `False` / `False` |
| TX_ERROR / RX_ERROR | `BrokenPipeError [Errno 32]` / `ConnectionResetError [Errno 104]` |
| Transfer result / process exit | `FAIL` / `1` |

The independent VPS target observed RX = 228,232,896 and TX = 228,232,896
bytes, followed by clean EOF. Historical wolves 1225 and 1226 remained
alive. The initiator restarted as PID 254615 with the same exact HC02 binary
SHA-256. Without manual Fang activation, the restarted daemon reported
lifecycle = `READY`, storage_degraded = false, and active_fangs = 1. The
restored Fang used local `127.0.0.1:17000`, transport `tcp`. A fresh session
echoed `STAGE22R-D02-TCP-RECOVERY` exactly (`MATCH=True`).

The interrupted transfer reported failure rather than false integrity;
persistent active-Fang intent and fresh TCP forwarding recovered. **D02
encrypted-TCP initiator-restart gate: PASS.**

## QUIC initiator restart

The Pack endpoint changed from TCP to QUIC. LC01 invalidated the affected
active Fang (`closed_fangs=1`), and the active list became empty. The operator
explicitly activated `wan-quic`: local `127.0.0.1:17001`, remote target
`127.0.0.1:7000`, transport `quic`. The precheck echoed
`STAGE22R-D02-QUIC-PRECHECK` exactly (`MATCH=True`).

The initiator was PID 254615, and its exact HC02 binary SHA-256 was
confirmed. SIGTERM interrupted an active 1 GiB transfer.

| Field | Observed |
|---|---|
| Expected bytes | 1,073,741,824 |
| TX bytes | 128,253,952 |
| RX bytes | 126,426,616 |
| TX SHA-256 | `3cf1bf5f411cf96757650514c60499e2c2ea4350e46685371f3bb993087d43a6` |
| RX SHA-256 | `5bb4120befce4be2ea08f360b88072e02011a0e206c91c71aef6aea5f2d30a27` |
| HASH_MATCH / SIZE_MATCH | `False` / `False` |
| TX_ERROR / RX_ERROR | `BrokenPipeError [Errno 32]` / `None` |
| Transfer result / process exit | `FAIL` / `1` |

The independent VPS target observed RX = 126,487,892 and TX = 126,487,892
bytes, followed by clean EOF. The historical wolves remained untouched.
The initiator restarted as PID 255309 with the same exact HC02 binary
SHA-256. Without manual Fang activation, it reported lifecycle = `READY`,
active_fangs = 1, protected-state generation = 27, and storage_degraded =
false. The automatically restored Fang was local `127.0.0.1:17001`, peer
`node-b`, remote target `127.0.0.1:7000`, transport `quic`, state `Active`.
A fresh session echoed `STAGE22R-D02-QUIC-RECOVERY` exactly (`MATCH=True`).

The interrupted transfer reported failure rather than false integrity;
persistent active-Fang intent and fresh QUIC forwarding recovered. **D02
QUIC initiator-restart gate: PASS.**

## Certification conclusion

Initiator SIGTERM mid-stream failed closed on both secure transports:
incomplete 1 GiB transfers never reported success, and transport failure
surfaced to the client. The historical daemons were unaffected. The same
HC02 binary was restored; persistent Pack, profile, and active-Fang state
survived; active Fang intent recovered automatically for both TCP and QUIC;
and fresh authenticated sessions succeeded. D02 initiator-restart behavior
is certified for both transports on the tested real-WAN path. This does not
claim interrupted transfers resume or that unsent bytes are delivered.
