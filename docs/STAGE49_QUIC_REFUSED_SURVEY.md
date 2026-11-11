# Stage49 — a QUIC peer with nothing listening (survey)

**Status: survey, for owner review.** Docs only. No production source,
harness, dependency or verdict changes. The probes below used scratch
scripts (a copy of the Stage46 sandbox with the Stage48 shapes, and a
UDP socket test), since removed; results in `target/stage49-survey/`.

Stage48 M1 found that a QUIC peer whose host is up but whose daemon is not
listening costs the full 5 s window, like a peer that never answers,
where TCP fails in milliseconds. This is the everyday "peer's daemon is
down" case. The survey measures it on every QUIC path and looks at what
can see the refusal. **Every QUIC path waits 5 s; one option makes it
fail at once on hosts that send ICMP, but it puts a new datagram on the
wire, so it needs owner review.**

## 1. Baseline

- `main` `b00bb54` (PR #154 merged, Stage48 CLOSED). Lab debug
  `werewolfd` `b928e6ff…`.
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18, quinn-udp 0.5.14,
  rustix 1.1.6.
- Tag `v1.0.0-rc.2` frozen. S3-P2 and S2-P4 stay EXCLUDED.

## 2. Measured (2026-11-11, Stage46 sandbox, `refused` = `127.0.0.2`, nothing listening)

| Path | QUIC | TCP |
|---|---|---|
| direct Fang (Stage48 M1) | reset, 5.0 s | reset, 0.00 s |
| failover, first provider refused (Stage48 M1) | next provider, 5.05–5.06 s | 0.01–0.02 s |
| failover, two refused, then live (Stage48 M1) | 10.05–10.06 s | 0.01–0.02 s |
| routed, hop to the relay | reset, 5.0 s; `dial` | reset, 0.00 s; `dial` |
| discovery `refresh` | `unreachable`, 5.01 s | `unreachable`, 0.01 s |
| governance `sync` | `unreachable`, 5.01 s | `unreachable`, 0.01 s |

A refused QUIC peer costs exactly what a silent one does (`blackhole`:
5.0 / 5.01 s on the same paths).

## 3. Why (CONFIRMED from code and a socket test)

- The kernel turns the peer's ICMP port-unreachable into `ECONNREFUSED`
  only on a **connected** UDP socket (or through `IP_RECVERR`'s error
  queue). Tested on this host: a connected socket to a closed port on
  `127.0.0.1` and `::1` sees `ECONNREFUSED` in 0.06–0.08 ms; an
  unconnected one sees nothing.
- quinn's endpoint socket is unconnected (one socket for every peer), so
  the refusal never reaches it. quinn also ignores `ECONNRESET` on receive
  by design ("undefined in QUIC and may be injected by an attacker",
  `quinn` 0.11.12 `endpoint.rs`), and treats any other receive error as
  fatal for the whole endpoint.

So nothing in quinn can be configured to see it; the signal has to come
from outside quinn.

## 4. Problems

- **P1 (CONFIRMED):** a QUIC peer with nothing listening costs 5 s per
  attempt on every path: each local client of a direct or routed Fang,
  each failover provider, each discovery or sync query.
- **P2:** within a hostname's addresses (Stage46 race), a refused QUIC
  address does not end its attempt early, so the next address starts on
  the 250 ms stagger instead of at once (TCP moves on at once). Small:
  Stage46 `quic:fast` is 0.31 s.

## 5. Options

- **R1, refusal probe.** With each QUIC dial attempt, open a separate UDP
  socket connected to the same address and send one 1-byte datagram. If
  it reports `ECONNREFUSED` before the attempt succeeds, end the attempt
  at once as a refused dial (`dial`, `unreachable`); otherwise drop the
  probe and let the QUIC dial run as now. Effects and costs:
  - a refused peer fails in milliseconds on every QUIC path, where the
    host sends ICMP (a LAN, the same host; many WAN paths filter ICMP,
    and there nothing changes);
  - **a new datagram on the wire:** a live receiver's quinn endpoint gets
    one 1-byte UDP datagram per dial attempt and must discard it (too
    short to be a QUIC packet); M1 would verify that it is dropped with no
    log or effect. Not a Werewolf message, no handshake or trust change,
    but observable, so it needs owner review;
  - ICMP is unauthenticated: an attacker who can forge an ICMP error for
    the probe's random source port can make one dial fail early. The
    outcome is the same as a dead peer (failover moves on, the client is
    reset); an on-path attacker can already drop the packets;
  - one more socket per QUIC attempt; if it cannot be opened (descriptor
    exhaustion) the attempt runs without a probe, as now.
- **R2, error queue on quinn's socket** (`IP_RECVERR`): rejected. quinn
  would see `ECONNREFUSED` on receive, which it treats as fatal for the
  endpoint, ending every connection on it.
- **R3, change quinn** (fork or upstream): a dependency change; not
  proposed.
- **R0, accept:** record P1 with the measured figures (it is the Stage45
  item, now measured on every path).

## 6. Recommendation

**R1, if the owner accepts the probe datagram; otherwise R0.** R1 is
the only way to see the refusal without changing quinn, it is
initiator-local, and its worst case is today's behaviour. The datagram
is the decision: it is new traffic towards every QUIC peer on every dial,
even when nothing is wrong.

## 7. Proposed Stage49 contract (for owner review)

- **M1, reproduction:** a tool (`tools/stage49/`, on the Stage46 sandbox
  with the Stage48 shapes) for QUIC `refused` on the direct, failover
  (one and two refused providers), routed, discovery and sync paths, with
  `blackhole` and `fast` as controls; and a check that a live receiver
  given 1-byte datagrams logs nothing and still serves sessions.
- **M2, fix (R1):** the probe in the QUIC dial attempt
  (`transport/dial.rs` race attempt for QUIC, `quic_fang`), unit tests:
  a closed UDP port fails the attempt at once as refused; a live or
  silent address is unaffected; a probe socket that cannot be opened is
  skipped. Criterion: QUIC `refused` under 0.1 s on every path;
  `blackhole` and `fast` unchanged.
- **M3, regression:** the M1 tool ×3; Stage46 matrix ×1; Stage48 tool ×1;
  Stage42 `fd:quic` floods (the extra socket under exhaustion); Stage32
  S2, S3, S4, S7 P0; `ci-local` full.
- **M4:** `KNOWN_LIMITATIONS` and closure.

No Werewolf wire message, handshake, trust, Target Policy or receiver
change under R1; the probe datagram is the one new thing on the wire.

## 8. Owner decisions

| ID | Decision | Options | Recommended |
|---|---|---|---|
| S49-D0 | Approve Stage49 as scoped (§7) | approve, amend, reject | approve |
| S49-D1 | Refused QUIC peer | R1 refusal probe (one 1-byte datagram per QUIC attempt); R0 accept and record | R1 if the datagram is acceptable, else R0 |
