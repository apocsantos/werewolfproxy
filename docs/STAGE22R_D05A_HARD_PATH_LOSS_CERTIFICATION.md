# Stage22R-D05A: real-WAN hard path-loss certification

Source baseline: `main` at `51a54d1fc8a9a9a9734eb6baa618d53384c0a043`.
At preparation, `origin/main` was `33ac83356d12ba03a49d8f437f43c3b42bab7911`.
This document records operator-reported real-WAN evidence. No production
code, protocol, release artifact, firewall setting, or network interface
configuration changed here. Historical local wolves were untouched.

## Purpose and fault method

D05A tests fail-closed behavior and path-only recovery when the WAN path
between the DarkSide initiator and Debian VPS receiver disappears abruptly
while both Werewolf daemons, the receiver service, and target
`127.0.0.1:7000` remain alive. A disposable relay outside the repository,
`/tmp/stage22r_chaos_proxy.py`, was the sole injected failure point. It was
automatically SIGKILLed after forwarding approximately 4 MiB; no Werewolf
process was killed. The relay is test infrastructure and is not part of this
repository.

## QUIC hard path loss

The relay mapped UDP `127.0.0.1:19560` to `51.83.41.28:9560`. During the
test, the initiator Pack endpoint was `quic://127.0.0.1:19560`. The active
Fang was ID `fang_6bb300c7b36756fc`, local `127.0.0.1:17001`, peer
`node-b`, remote target `127.0.0.1:7000`, transport `quic`, state `Active`.
The precheck echoed `STAGE22R-D05A-QUIC-PROXY-PRECHECK` exactly (`MATCH=True`).
The initiator was PID 263850 with SHA-256
`8dad5819d99fac16b5519092e8f38da0d3aa8d3c681aa024925706d8c43e13fc`.

Relay PID 309798 started at `2026-09-25T09:29:19.730134896+01:00` and
recorded a UDP map from client `127.0.0.1:52663` to upstream
`51.83.41.28:9560`.

| Event | Time (`+01:00`) |
|---|---|
| Client start | `2026-09-25T09:30:13.027094957+01:00` |
| Hard path cut | `2026-09-25T09:30:16.731042584+01:00` |
| Client end | `2026-09-25T09:30:46.740342118+01:00` |

The relay logged `TRIP kind=udp`, `forwarded_bytes=4194726`,
`threshold=4194304`, `action=SIGKILL`; its process was confirmed dead.

| Client field | Observed |
|---|---|
| Expected bytes | 1,073,741,824 |
| TX bytes | 5,898,240 |
| RX bytes | 3,996,976 |
| TX SHA-256 | `ad1843c4c6b3c3116b5883f9ed118bba26ed1dfed5b6d62b959cbcf1468a62dd` |
| RX SHA-256 | `745132a698de1e173c72350660fb8b445da8c84bf816cd6610023e5da0676a74` |
| HASH_MATCH / SIZE_MATCH | `False` / `False` |
| TX_ERROR / RX_ERROR | `BrokenPipeError [Errno 32]` / `None` |
| Transfer result / process exit | `FAIL` / `1` |

The client detected failure approximately 33.7 seconds after its start and
approximately 30 seconds after the relay cut. These are observations, not
an attribution to a particular QUIC timer. The independent target recorded
connection port 42020, RX = 4,069,612 and TX = 4,069,612 bytes,
elapsed = 33.596680 s, and EOF.

After the fault, the VPS receiver remained `active` / `running` with
MainPID = 25743 and `NRestarts=4`. Its state was lifecycle = `READY`,
`inbound_authority.active=0`, `inbound_authority.healthy=true`, locked =
false, storage_degraded = false, packmates = 1, and protected-state
generation = 16. No systemd entries occurred during the D05A QUIC interval.

For recovery, **only** the UDP relay was restarted on the same endpoint,
with `trip-kind=none`. No Werewolf daemon was restarted, no `pack set-address`
was performed, and no `fang activate` was performed. The Fang stayed
`Active`. A fresh session echoed `STAGE22R-D05A-QUIC-PATH-RECOVERY` exactly
(`MATCH=True`), and the recovery relay recorded a new `UDP_MAP`.
**D05A QUIC hard path loss: PASS.**

## Encrypted TCP hard path loss

The relay mapped TCP `127.0.0.1:18443` to `51.83.41.28:8443`; the
initiator Pack endpoint was `tcp://127.0.0.1:18443`. The endpoint change
invalidated the previous QUIC Fang under LC01 before TCP activation. The
active TCP Fang was ID `fang_fda3fd81c8f71319`, local `127.0.0.1:17000`,
peer `node-b`, remote target `127.0.0.1:7000`, transport `tcp`, state
`Active`. The precheck echoed `STAGE22R-D05A-TCP-V2-PRECHECK` exactly
(`MATCH=True`). The initiator remained PID 263850 with the HC02 SHA-256
listed above.

Valid relay PID 312700 started at `2026-09-25T10:25:42.291422165+01:00`.
The large-transfer TCP connection began at
`2026-09-25T10:26:49.797392424+01:00`.

| Event | Time (`+01:00`) |
|---|---|
| Client start | `2026-09-25T10:26:49.704066423+01:00` |
| Hard path cut | `2026-09-25T10:26:52.953798350+01:00` |
| Client end | `2026-09-25T10:26:52.964193896+01:00` |

The relay logged `TRIP kind=tcp`, `forwarded_bytes=4206416`,
`threshold=4194304`, `action=SIGKILL`; its process was confirmed dead.

| Client field | Observed |
|---|---|
| Expected bytes | 1,073,741,824 |
| TX bytes | 5,177,344 |
| RX bytes | 3,082,992 |
| TX SHA-256 | `f312abafacf52315d32d293b412817fc96385293ecc2d875e27b3afd2242b3ee` |
| RX SHA-256 | `fc7c6712b215aac150a5cc33cea792a54d6f51133919b370db70f2cf21f689f3` |
| HASH_MATCH / SIZE_MATCH | `False` / `False` |
| TX_ERROR / RX_ERROR | `BrokenPipeError [Errno 32]` / `ConnectionResetError [Errno 104]` |
| Transfer result / process exit | `FAIL` / `1` |

TCP failure surfaced approximately 3.2 seconds after client start, almost
immediately after relay SIGKILL. The independent target recorded connection
port 60972, RX = 3,308,600 and TX = 3,308,600 bytes,
elapsed = 2.976695 s, and EOF.

After the fault, the VPS receiver remained `active` / `running` with
MainPID = 25743 and `NRestarts=4`. Its state was lifecycle = `READY`,
`inbound_authority.active=0`, `inbound_authority.healthy=true`, locked =
false, storage_degraded = false, packmates = 1, and protected-state
generation = 16. No systemd entries occurred during the D05A TCP interval.
The initiator remained PID 263850 with the same HC02 SHA-256, and its Fang
remained `Active`.

For recovery, **only** the TCP relay was restarted on the same
`127.0.0.1:18443` endpoint, as PID 313100. No Werewolf daemon restarted, no
`pack set-address` occurred, and no `fang activate` occurred. A fresh
session echoed `STAGE22R-D05A-TCP-PATH-RECOVERY` exactly (`MATCH=True`);
this success was repeated twice. The recovery relay showed fresh
`TCP_CONNECT` / `TCP_EOF` cycles. **D05A encrypted-TCP hard path loss: PASS.**

### Excluded TCP attempt

An earlier attempt is **not D05A certification evidence**. A second
recovery relay failed with `OSError: [Errno 98] Address already in use`
because the original destructive TCP relay was still bound to
`127.0.0.1:18443`; the 1 GiB destructive test had not yet started. A
subsequent small recovery request therefore passed through that still-live
original relay. Both disposable relay processes were explicitly terminated,
and the valid V2 TCP test above began from a clean listener state.

## Certification conclusion

Abrupt loss of the network path did not require Werewolf process death:
initiator and receiver remained alive, the receiver systemd restart counter
did not change, protected state remained healthy, and the active Fang
survived. Each incomplete 1 GiB transfer failed visibly rather than
reporting success; the target received partial application data and the
local application observed transport failure. Restoring only the relay path
was sufficient for fresh authenticated sessions, without daemon restart,
Pack mutation during recovery, or Fang reactivation.

Encrypted TCP surfaced relay loss in roughly 3.2 seconds; QUIC surfaced it
in roughly 33.7 seconds. These values describe only this topology and test
method; no specific protocol timeout is claimed. Encrypted TCP: **PASS**.
QUIC: **PASS**. Stage22R-D05A: **CERTIFIED**.
