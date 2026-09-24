# Stage22R-D04: real-WAN receiver hard-crash certification

Source baseline: `main` at `33ac83356d12ba03a49d8f437f43c3b42bab7911`.
This document records operator-reported real-WAN evidence. No production
code, protocol, service configuration, or release artifact changed here.

## Purpose, topology, and receiver identity

D04 tests fail-closed behavior and supervised recovery when a Debian VPS
receiver is killed with SIGKILL during an active authenticated transfer
from the DarkSide initiator over the real WAN. The target was
`127.0.0.1:7000`; only encrypted TCP and QUIC were tested.

The receiver executable was `/usr/local/bin/werewolfd`, SHA-256
`ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4`.
The `werewolfd.service` systemd unit used `Restart=on-failure`,
`RestartSec=5s`, `StartLimitIntervalSec=60s`, and `StartLimitBurst=5`.
Its receiver command was:

```text
/usr/local/bin/werewolfd --home /var/lib/werewolfproxy \
  --socket /run/werewolfproxy/control.sock \
  --listen 0.0.0.0:8443 --quic-listen 0.0.0.0:9560
```

## Certification method and excluded attempts

Some earlier, manually timed SIGKILL attempts occurred only after the large
transfer had terminated. They demonstrated systemd restart behavior but
are **excluded** from D04 mid-transfer certification. Only the automated
authority-triggered runs below are certification evidence. Their method was:

1. Verify `inbound_authority.active == 0`, arm a receiver-side watcher,
   and start a 1 GiB transfer from DarkSide.
2. Poll receiver authority; on `inbound_authority.active >= 1`, capture
   receiver PID, exact executable, SHA-256, and `NRestarts`, then issue
   SIGKILL immediately.
3. Observe systemd restart and verify a new PID, the same executable/hash,
   and active/running service state.
4. Verify the client reports transfer failure, the target received only
   partial data, and a fresh authenticated session succeeds afterward.

## Encrypted TCP receiver hard crash

The initiating Fang was ID `fang_33e199bd43064a7a`, local
`127.0.0.1:17000`, peer `node-b`, remote target `127.0.0.1:7000`, transport
`tcp`, state `Active`. The precheck echoed `STAGE22R-D04-TCP-PRECHECK`
exactly (`MATCH=True`).

| Event | Timestamp |
|---|---|
| Client start | `2026-09-24T14:14:32.380118723+01:00` = `2026-09-24T13:14:32.380118723Z` |
| Authority active observed | `2026-09-24T13:14:32.759319068Z` |
| SIGKILL | `2026-09-24T13:14:32.981182328Z` |
| Client end | `2026-09-24T14:14:33.088335686+01:00` = `2026-09-24T13:14:33.088335686Z` |
| Receiver restarted | `2026-09-24T13:14:38.230804011Z` |

Immediately before the kill, receiver PID 16916 resolved to
`/usr/local/bin/werewolfd` with the exact SHA-256 above;
`NRESTARTS_BEFORE=2` and `inbound_authority.active=1`. SIGKILL followed
authenticated active-session publication.

| Client field | Observed |
|---|---|
| Expected bytes | 1,073,741,824 |
| TX bytes | 2,490,368 |
| RX bytes | 49,000 |
| TX SHA-256 | `7226401040600582233d8fc3984e3b8ce9d31161445c2a13eec9a50934b58764` |
| RX SHA-256 | `89a983112c869c77043950770ec64b535ca7647b8f0d5092054142eef14a3383` |
| HASH_MATCH / SIZE_MATCH | `False` / `False` |
| TX_ERROR / RX_ERROR | `ConnectionResetError [Errno 104] Connection reset by peer` / `None` |
| Transfer result / client process exit | `FAIL` / `1` |

The independent target recorded connection port 46024, a
`ConnectionResetError`, RX = 323,448 and TX = 323,448 bytes, elapsed =
0.297050 s. The abrupt receiver death reached the target-side socket;
the incomplete transfer was not reported as successful.

Systemd restarted the receiver from PID 16916 to PID 20736, with
`NRestarts` increasing exactly once from 2 to 3. The service returned to
`active` / `running` with the same binary SHA-256. Receiver state afterward:
lifecycle = `READY`, healthy = true, locked = false,
storage_degraded = false, packmates = 1, protected-state generation = 16,
and `inbound_authority.active=0`. The initiator Fang remained active without
manual reactivation. A fresh session echoed `STAGE22R-D04-TCP-RECOVERY`
exactly (`MATCH=True`). **D04 encrypted-TCP receiver hard crash: PASS.**

## QUIC receiver hard crash

The Pack endpoint changed to `quic://51.83.41.28:9560`. LC01 invalidated
the affected Fang (`closed_fangs=1`), leaving the active Fang list empty.
The operator explicitly activated `wan-quic` before testing. The initiating
Fang was ID `fang_3fdb67b923f4b86f`, local `127.0.0.1:17001`, peer
`node-b`, remote target `127.0.0.1:7000`, transport `quic`, state `Active`.
The precheck echoed `STAGE22R-D04-QUIC-PRECHECK` exactly (`MATCH=True`).

| Event | Timestamp |
|---|---|
| Client start | `2026-09-24T14:42:53.439285288+01:00` = `2026-09-24T13:42:53.439285288Z` |
| Authority active observed | `2026-09-24T13:42:53.643450272Z` |
| SIGKILL | `2026-09-24T13:42:53.816885435Z` |
| Receiver restarted | `2026-09-24T13:42:59.219585619Z` |
| Client end | `2026-09-24T14:43:23.834236492+01:00` = `2026-09-24T13:43:23.834236492Z` |

Immediately before the kill, receiver PID 20736 resolved to
`/usr/local/bin/werewolfd` with the exact SHA-256 above;
`NRESTARTS_BEFORE=3` and `inbound_authority.active=1`.

| Client field | Observed |
|---|---|
| Expected bytes | 1,073,741,824 |
| TX bytes | 1,966,080 |
| RX bytes | 9,072 |
| TX SHA-256 | `3ed1be340852444cf92ae4699c83216fee1f0458325d9d329a7c403b34862794` |
| RX SHA-256 | `d2abb277fcc6267bcae6614ce6d53d4ed0c08dfff08e79cb0d19560ec16da856` |
| HASH_MATCH / SIZE_MATCH | `False` / `False` |
| TX_ERROR / RX_ERROR | `BrokenPipeError [Errno 32] Broken pipe` / `None` |
| Transfer result / client process exit | `FAIL` / `1` |

The independent target recorded connection port 48378, RX = 9,072 and
TX = 9,072 bytes, elapsed = 0.246730 s, and EOF. No false success occurred.

Systemd restarted the receiver from PID 20736 to PID 25743, with
`NRestarts` increasing exactly once from 3 to 4. The service returned to
`active` / `running` with the same receiver SHA-256. Receiver state afterward:
lifecycle = `READY`, `inbound_authority.active=0`,
`inbound_authority.healthy=true`, locked = false, storage_degraded = false,
packmates = 1, protected-state generation = 16, and both TCP and QUIC
listener states = `configured`. The initiator Fang remained active as
`fang_3fdb67b923f4b86f`, local `127.0.0.1:17001`, transport `quic`, state
`Active`. No manual `fang activate` was performed after the receiver crash.
A fresh session echoed `STAGE22R-D04-QUIC-RECOVERY` exactly (`MATCH=True`).

The TCP client surfaced receiver death almost immediately. The QUIC client
terminated approximately 30 seconds after SIGKILL. This is an observed
transport difference only; D04 does not attribute it to a specific timeout
or implementation mechanism. **D04 QUIC receiver hard crash: PASS.**

## Certification conclusion

For both secure transports, receiver identity and binary were verified
immediately before SIGKILL, and session authority was active. The receiver
was killed non-gracefully during a live transfer; neither partial 1 GiB
transfer falsely reported success, and transport failure reached the local
application. The target observed bounded partial data. Systemd treated
SIGKILL as a failure under `Restart=on-failure`, restarted the same receiver
binary automatically, and incremented `NRestarts` exactly once per certified
crash. Protected state remained healthy, the receiver returned to `READY`
without manual state repair, the initiator Fang stayed active without
reactivation, and fresh authenticated sessions succeeded. Encrypted TCP:
**PASS**. QUIC: **PASS**. Stage22R-D04: **CERTIFIED** on this real-WAN path.
