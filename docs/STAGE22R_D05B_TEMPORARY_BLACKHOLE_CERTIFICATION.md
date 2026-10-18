# Stage22R-D05B: real-WAN temporary blackhole certification

Source baseline: `main` at `dada25d8c8bc7b42ca044d2857e56890aa5ef6c2`.
Runs executed on 2026-10-18 (times `+01:00`). This document records
real-WAN evidence from the DarkSide initiator and operator-reported evidence
from the Debian VPS receiver. No production code, protocol, release
artifact, firewall setting or network interface configuration changed here.
Historical local wolves (PIDs 1270 `wolf-a`, 1273 `wolf-b`) were untouched.

## Purpose and fault method

D05A removed the path permanently. D05B asks whether an **already
established authenticated session** survives a temporary outage: the relay
process and its sockets stay alive, forwarding stops for N seconds, then
resumes in the same process. Valid outcomes are (A) the same session
survives with exact 1 GiB integrity, or (B) the session is declared dead,
the transfer fails visibly, and a fresh session works afterwards.

Disposable test infrastructure outside the repository:

| File | SHA-256 |
|---|---|
| `/tmp/stage22r_chaos_proxy.py` (relay, `--trip-kind pause`) | `5e99709c0d96c5125bde95347a0bbf78b4249e54311a35203844990f7639e866` |
| `/tmp/stage22r_duplex_client.py` (1 GiB full-duplex echo client) | `eb71c0aa4fd26419def19fe2e8f42bca7acef921b74b28fe866b68d01fe9e196` |
| `/tmp/stage22r-d05b/run.sh` (per-run driver) | `b2e3c28e154c7dca8ea20053e717596575c1564aae98e95be8976d1221c7f783` |

Each run started a fresh relay, sent an exact-echo precheck marker, then
one 1 GiB transfer of fresh random data on **one** TCP connection to the
local Fang, reading the echo concurrently and comparing size and SHA-256 of
everything sent and received. The client never reconnects; its 180 s
receive-idle limit was never reached, so the transport alone decided
session survival. The relay tripped after ~4 MiB forwarded (both
directions summed) and logged `TRIP`, `PAUSE_START` and `PAUSE_END` with
wall and monotonic timestamps and per-direction counters. After a failed
transfer the driver sent a recovery marker on a fresh session.

Pause semantics differ by transport and bound what each result means:

- **QUIC (UDP):** the relay kept reading and **discarded** datagrams in both
  directions during the pause, as a real blackhole does.
- **Encrypted TCP:** the relay **stopped reading and writing** and kept both
  sockets open; no byte was discarded (`dropped=0`). Both TCP connections
  (Fang↔relay on loopback, relay↔VPS on the WAN) saw a stall with full
  windows, not packet loss, so TCP retransmission backoff was not exercised.
  This measures Werewolf's tolerance of a stalled TCP session, not of WAN
  packet loss.

## Topology and nodes

DarkSide initiator: HC02 `werewolfd` PID 2377544, SHA-256
`8dad5819d99fac16b5519092e8f38da0d3aa8d3c681aa024925706d8c43e13fc`,
Den `~/.werewolf-stage22r`, started for this test with
`--listen 127.0.0.1:28443 --quic-listen 127.0.0.1:29560` (wolf-a/wolf-b
hold 8443/9443/9560/9561). Control binary SHA-256
`ac832c0f050e7c09d750dcef7511ac0b8b7d551a0bb949ba4dfb75fa1c758e84`.
The same PID and SHA-256 were recorded before, after and at the end of
every run, with lifecycle `READY`, `storage_degraded=false`,
`inbound_authority.active=0` and the active Fang `Active`.

VPS receiver: `werewolfd.service` MainPID 25743, `NRestarts=4` before the
series and after every reported check; lifecycle `READY`, protected-state
generation 16, `storage_degraded=false`, packmates 1. Target
`python3 /tmp/werewolf-echo-trace.py` on `127.0.0.1:7000`, PID 4481,
running since 2026-09-23 and not restarted. Its log file had been deleted
while open and was read through `/proc/4481/fd/1`.

Pack and Fang changes happened only between runs, never during one:

| Series | Pack `node-b` | Relay | Fang | DarkSide generation during runs |
|---|---|---|---|---|
| QUIC | `quic://127.0.0.1:19560` | UDP `127.0.0.1:19560` → `51.83.41.28:9560` | `wan-quic` `fang_d201e91ad2d9b37d`, local `127.0.0.1:17001` | 37 |
| TCP | `tcp://127.0.0.1:18443` | TCP `127.0.0.1:18443` → `51.83.41.28:8443` | `wan-tcp` `fang_97720db79fff178e`, local `127.0.0.1:17000` | 39 |

Each `pack set-address` closed the previous Fang (`closed_fangs=1`, LC01)
before the next profile was activated.

## QUIC results

Every precheck matched exactly. Each relay saw a single UDP client (one
QUIC 4-tuple) for its whole run.

| Pause | Relay PID | `PAUSE_START` → `PAUSE_END` | Dropped c2u / u2c | TX / RX bytes | Outcome |
|---|---|---|---|---|---|
| 5 s | 2377697 | 17:36:06.432 → 17:36:11.432 (5.0004 s) | 7,846 / 60,475 | 1,073,741,824 / 1,073,741,824 | A: same session, exact |
| 10 s | 2429936 | 17:55:13.858 → 17:55:23.858 (10.0005 s) | 9,237 / 62,225 | 1,073,741,824 / 1,073,741,824 | A: same session, exact |
| 20 s | 2535136 | 18:27:56.905 → 18:28:16.907 (20.0011 s) | 7,428 / 56,794 | 1,073,741,824 / 1,073,741,824 | A: same session, exact |
| 22 s | 2713319 | 20:31:11.626 → 20:31:33.626 (22.0005 s) | 10,711 / 58,001 | 3,801,088 / 1,994,654 | B: ended 20:31:41.629 |
| 25 s | 2710136 | 20:24:39.025 → 20:25:04.026 (25.0006 s) | 16,767 / 51,786 | 3,801,088 / 1,996,556 | B: ended 20:25:09.028 |
| 30 s | 2708569 | 20:13:38.979 → 20:14:08.980 (30.0006 s) | 10,616 / 57,937 | 3,801,088 / 1,994,918 | B: ended 20:14:08.981 |

The 22 s run was executed after the 25 s and 30 s runs to bracket the limit.

| Pause | Client duration | Longest RX gap | TX SHA-256 | RX SHA-256 |
|---|---|---|---|---|
| 5 s | 977.457 s | 6.028 s | `2f6153c6f109a987f8b5c6c397127aeaabcd686eea989d2531ca659e325b2d5d` | identical |
| 10 s | 940.863 s | 10.291 s | `4cf2cf15144fd3c6acf37aef3580bc2941a36766718cd308ce574c777d88425f` | identical |
| 20 s | 1,031.436 s | 24.812 s | `543d391d2b135e2a7708d6544e2584d2fc64a140919ffb2866a76be9c3b2caac` | identical |
| 22 s | 32.050 s | n/a | `f1153f27ecdc5639a5bf2327be0d146eec83d864d09787f31c123a54be56a916` | `3bc72249b0b8e8b970e403b672c03eb64028e9e8ec1ccdcb661fd160cb219906` |
| 25 s | 32.096 s | n/a | `9113181b809ee89183a44c74b8584fb0d1c4fe8060a5fd238b22338f5c02305c` | `2316746c17ad4f554b3ada343a4b8be0e1811d3bf0a03d10692c5c4969460638` |
| 30 s | 32.074 s | n/a | `f2ffa55ed4f5cbc3f3a2155f4b9313cea4dceb14e70b0274583db8a0b21110ed` | `16c8478da1546c8a0d93fe6f9dc152452c090a5487cedbbca89256b0c3fff9de` |

In the three failed runs the client reported `SIZE_MATCH=False`,
`HASH_MATCH=False`, `FAIL`, exit 1, TX `BrokenPipeError [Errno 32]`, and
receive EOF. The recovery markers `STAGE22R-D05B-QUIC-{22S,25S,30S}-RECOVERY`
echoed exactly on a fresh session within one second, with no daemon restart,
no Pack mutation and no Fang reactivation. (`max_rx_gap_s` excludes the
final silence before EOF, so it is not meaningful for failed runs.)

VPS target, one connection per transfer, each ending in EOF:

| Pause | Target RX / TX | Target elapsed |
|---|---|---|
| 5 s | 1,073,741,824 / 1,073,741,824 | 977.341 s |
| 10 s | 1,073,741,824 / 1,073,741,824 | 940.753 s |
| 20 s | 1,073,741,824 / 1,073,741,824 | 1,031.336 s |
| 22 s | 2,050,258 / 2,050,258 | 31.988 s |
| 25 s | 2,046,249 / 2,046,249 | 32.029 s |
| 30 s | 2,050,514 / 2,050,514 | 31.997 s |

Prechecks and recoveries appear as 32-byte connections. In failed runs the
target echoed more than the client received; the difference (about 55 KB)
matches the VPS→DarkSide datagrams discarded during the pause. No byte was
duplicated or reported as success.

Every failed session ended **30.0 s after forwarding stopped**, matching
`QUIC_IDLE_TIMEOUT = 30 s` (`crates/werewolfd/src/transport/quic.rs:35`).
In the 22 s and 25 s runs the path was already restored for 8 s and 5 s
respectively, yet the session did not resume; after a 20 s pause it resumed
4.8 s after the path returned. This is consistent with QUIC loss-probe
backoff placing the next probe beyond the idle timeout, but the relay does
not log individual datagrams, so the mechanism is not proven here.

## Encrypted TCP results

Every precheck matched exactly on its own relay connection (conn 1). Each
transfer used one relay TCP connection (conn 2) from accept to close, and the
relay discarded no bytes.

| Pause | Relay PID | `PAUSE_START` → `PAUSE_END` | TX / RX bytes | Client duration | Longest RX gap | SHA-256 (TX = RX) |
|---|---|---|---|---|---|---|
| 5 s | 2719734 | 20:36:51.288 → 20:36:56.289 (5.0004 s) | 1,073,741,824 / 1,073,741,824 | 890.417 s | 4.550 s | `e7056829eda550641d9d0a231c2938ae45472a5521815c5db75cb50e7f601d98` |
| 10 s | 2751729 | 20:52:02.842 → 20:52:12.843 (10.0005 s) | 1,073,741,824 / 1,073,741,824 | 1,099.064 s | 9.798 s | `8e86e59c11cf0ab0903a5e2cbcd18f498c681aa3e88a060402b9a6faaba3b6d0` |
| 20 s | 2884276 | 21:10:36.351 → 21:10:56.352 (20.0006 s) | 1,073,741,824 / 1,073,741,824 | 927.814 s | 19.965 s | `64702dc26be6abb3206c2d0264a25dc9629a91282a33a65bb8060968657d3a1d` |
| 30 s | 2935139 | 21:26:18.542 → 21:26:48.542 (30.0005 s) | 1,073,741,824 / 1,073,741,824 | 777.031 s | 29.639 s | `69f8d6945aaf43d533f1b84ede227183feecf94c104e4d149bc00892d7e087f1` |

All four transfers: `SIZE_MATCH=True`, `HASH_MATCH=True`, no errors, clean
EOF, exit 0. Outcome A in every case; transfer resumed as soon as the relay
resumed. The relay carried about 1,353 MB per direction for 1,074 MB of
payload (~26 % encrypted-TCP framing overhead on the wire).

VPS target, one connection per transfer, each ending in EOF:

| Pause | Target RX / TX | Target elapsed |
|---|---|---|
| 5 s | 1,073,741,824 / 1,073,741,824 | 890.140 s |
| 10 s | 1,073,741,824 / 1,073,741,824 | 1,098.752 s |
| 20 s | 1,073,741,824 / 1,073,741,824 | 927.497 s |
| 30 s | 1,073,741,824 / 1,073,741,824 | 776.684 s |

After the TCP series the VPS receiver was still MainPID 25743 with
`NRestarts=4`, lifecycle `READY`, `inbound_authority.active=0`,
`healthy=true`, `locked=false`, `storage_degraded=false` and generation 16.

## Certification conclusion

No temporary outage produced a false success, a daemon restart, storage
degradation or retained inbound authority. Every surviving session
delivered exactly 1 GiB with matching SHA-256; every failed session was
reported as a failure and a fresh authenticated session worked immediately
without restart, Pack mutation or Fang reactivation.

- **QUIC:** the same session survives blackholes up to 20 s; at 22 s and
  above it ends at the 30 s idle timeout. The observed boundary on this path
  lies between 20 s and 22 s of blackhole. **PASS.**
- **Encrypted TCP:** the same session survives stalls of 5–30 s with exact
  integrity; no application-level stall timeout of 30 s or less exists.
  Packet-loss blackholes, which would exercise TCP retransmission backoff,
  were not tested (they require firewall changes outside this method).
  **PASS.**

Stage22R-D05B: **CERTIFIED** (QUIC and encrypted TCP).

Observations for follow-up, not defects found by this test:

- When a QUIC session dies, the local application sees receive EOF before
  the send error; an application that treats EOF as completion without its
  own length or integrity check could mistake truncation for success.
- The QUIC survival boundary (20–22 s) is shorter than the 30 s idle
  timeout because of probe backoff; QUIC keep-alive or idle-timeout tuning
  would change it and is a protocol decision, not part of this record.
