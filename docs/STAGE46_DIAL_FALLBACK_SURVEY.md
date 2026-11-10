# Stage46 — dialling every resolved address (survey)

**Status: survey, for owner review.** Docs only. No production source,
harness, dependency or verdict changes.

Stage44 recorded (S44-D4) and Stage45 kept (H1) one limitation: a peer
whose Pack address is a hostname is dialled over QUIC at its **first**
resolved address only. On the test host `localhost` resolves to `::1`
first, so a QUIC Fang to `localhost` fails every session against an
IPv4-only listener while a TCP Fang to the same address works. Stage45
made that failure visible; this survey reads every place a peer address is
dialled and proposes how to try the others. It also takes the labelling
flaw Stage45 M3 found (a local resource error on the dial is labelled
`handshake`), because the fix sits in the same dial code.

## 1. Baseline

- `main` `4465894` (PR #138 merged, Stage45 CLOSED).
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18.
- Tag `v1.0.0-rc.2` frozen. S3-P2 and S2-P4 stay EXCLUDED.
- Test host: glibc 2.41, `/etc/host.conf` `multi on`. `/etc/hosts` lists
  `127.0.0.1 localhost` before `::1 localhost`, yet `getent ahosts
  localhost` answers `::1` first: `getaddrinfo` sorts by RFC 6724, so the
  file order does not decide.

## 2. Where a peer address is dialled (CONFIRMED from code)

A Pack address is `quic://HOST:PORT` or `tcp://HOST:PORT`, where HOST may
be a hostname (`state_validation.rs` checks only a non-empty host and a
non-zero port). Contact records and published service targets are literal
IPs only (`werewolf-core` `contact.rs`, `published.rs`), so a hostname can
enter only through `pack add` or `pack set-address`.

| Path | Where | Hostname handling today |
|---|---|---|
| QUIC Fang | `main.rs` `resolve_quic_peer` → `quic_fang::resolve_server` | resolved once at open (H1), **first address**; the Fang's QUIC endpoint is bound for that address's family |
| encrypted-TCP Fang | `tcp_encrypted.rs` `open_tcp`: `TcpStream::connect(&str)` in a 5 s timeout | resolved **per session**; tokio tries **every** address in order and returns the last error |
| plain-TCP Fang (target) | `tcp_plain.rs`: `TcpStream::connect(&remote)`, 5 s | as encrypted TCP |
| failover Fang, QUIC provider | `failover.rs` `Path::Quic(address.parse())` | **refused at open** (`FANG_BAD_QUIC_ADDRESS`) |
| failover Fang, TCP provider | `connect_tcp_session(&str)` | as encrypted TCP |
| routed Fang, QUIC hop to relay | `relay.rs:207`, `:301`: `routed.relay_address.parse()` **per session**; the endpoint is created with `parse().ok()` = `None`, so IPv4 | opens, then **every session fails**; the error is `hs::rejected()`, so Stage45 logs and counts it as `handshake` |
| routed Fang, TCP hop to relay | `connect_authenticated_tcp(&relay_address)` | as encrypted TCP |
| routed destination | `relay.rs` `resolve` | must be a literal IP (it is the inner TLS name); refused at open otherwise. By design |
| discovery, sync, `service add --from`, over QUIC | `discovery/operator.rs` (three sites): `lookup_host`, **first address** | per query; the whole query is bounded by the operator deadline |
| discovery and sync over TCP | `rpc_tcp(&str)` → `TcpStream::connect` | every address, in order |

So the same hostname behaves four ways: TCP tries every address; a direct
QUIC Fang, discovery and sync try the first; a failover QUIC provider is
refused at open; a routed QUIC hop opens and then fails every session.

**TCP's fallback is partial.** tokio's `TcpStream::connect(&str)` tries
the addresses one after another. A refused or unroutable first address
fails fast and the next is tried. A first address that drops packets
silently holds the connect until the 5 s timeout around it, so the second
address is never tried.

**The QUIC endpoint is per family.** Stage44 (S44-D1, V1) binds a QUIC
client endpoint to `0.0.0.0:0` or `[::]:0` by the destination's family,
so a host with IPv6 disabled is unaffected. A Fang has one endpoint for
its lifetime. Trying an address of the other family needs a second
endpoint.

**Identity does not depend on the name.** Every dial pins the peer's Pack
key; the TLS server name is the IP literal dialled (SNI suppressed for
QUIC, the socket's peer IP for TCP). Trying another address of the same
name cannot reach a different identity: a wrong host fails the handshake
as `handshake`. No trust question arises.

## 3. Problems

- **D1, first address only over QUIC.** A direct QUIC Fang, discovery,
  sync and `service add --from` over QUIC fail when the first address is
  wrong, though another address works. `localhost` on this host is the
  everyday case.
- **D2, routed QUIC hop to a hostname relay** opens and then fails every
  session, labelled `handshake` (a Stage45 P5 left on another path).
- **D3, failover QUIC provider with a hostname** is refused at open, unlike
  a direct QUIC Fang to the same peer.
- **D4, a silent first address costs TCP its whole window.** The fallback
  happens only when the first address fails fast.
- **D5, local resource errors labelled `handshake`** (Stage45 M3 §3):
  `EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM` on the dial carry no dial
  `ErrorKind`. `client_endpoint` also maps every endpoint error,
  `EMFILE` included, to `hs::rejected()`. Discovery reports these
  `rejected`.

## 4. Options

### Addresses: when to resolve (QUIC)

- **A1, resolve at open, keep every address.** H1 as approved, but the
  Fang keeps the whole list, in resolver order. A DNS change is followed on
  reopen, as now.
- **A2, resolve per session**, as TCP does. Follows DNS changes; a lookup
  per session (bounded by the window), and a resolution failure appears
  per session instead of at open.

### How to try them (QUIC and, optionally, TCP)

- **R1, staggered race** (RFC 8305 §5, without its DNS part). Start with
  the first address; if it has not connected after 250 ms, start the next
  as well, alternating families; the first handshake to complete wins and
  the others are closed. The whole dial stays inside the existing window
  (`hs::READ_WINDOW`, 5 s). A silent first address costs 250 ms.
- **R2, sequential.** Next address only after a failure, all inside the
  same window. Simple; a silent first address still uses the whole window
  (D4), so it fixes `localhost` → `::1` only where `::1` fails fast.
- **R0, record only** (today).

R1 on QUIC sends an Initial to a second address while the first may still
answer; the loser is closed with no data sent on it. The receiver sees a
connection that closes after its handshake; the per-peer quota counts
sessions, not handshakes, so it is not charged. To be confirmed in M1.

### Endpoints for the other family

- **E1, one endpoint per family, created on first use** by the Fang. A host
  with IPv6 disabled never creates the IPv6 one unless an IPv6 address is
  tried, and then that attempt fails as `dial` and the IPv4 one proceeds.
- **E2, a dual-stack `[::]:0` endpoint** (rejected as V1b in Stage44).

### Which paths

- **P-all:** direct QUIC Fang, failover QUIC providers (resolve at open,
  D3), routed QUIC hops (resolve the relay at open, D2), discovery, sync
  and `service add --from`, through one shared dialer.
- **T1:** TCP dials also use the shared dialer (fixes D4).
- **T0:** TCP keeps tokio's sequential connect.

### Local resource errors (D5)

- **L1, `dial`.** Classify `EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM` by raw
  OS error as `dial` (no connection was made); keep the error kind of a
  failed endpoint creation instead of `hs::rejected()`; discovery and sync
  then report `unreachable`, as their contract defines it ("no connection
  was made").
- **L2, a new class `local`** in the log, `last_failure` and a new
  discovery status. More precise; a contract addition.
- **L0, leave** (as recorded).

## 5. Recommendation

**A1 + R1 + E1 + P-all + T1 + L1.**

- A1 keeps the open-time behaviour the owner approved in H1 and adds no
  per-session lookups.
- R1 is the only option that covers a silent first address, which is the
  case a real dual-stack name with a broken IPv6 path produces. 250 ms
  is the RFC 8305 default.
- E1 keeps Stage44's rule that IPv4-only hosts never touch IPv6.
- P-all removes the four behaviours of §2 in favour of one: every dial of a
  Pack hostname tries every address, inside the window it has today.
- T1 makes TCP match; it changes how TCP dials (concurrent attempts after
  250 ms) and so is the riskiest part. T0 is the fallback if the owner
  wants TCP untouched.
- L1 needs no contract change.

Out of scope: the 5 s cost per client of a dead QUIC peer (Stage45),
re-resolving while a Fang is open, SRV records, a hostname as a routed
destination (by design a literal), contact records with hostnames.

## 6. Proposed Stage46 contract (for owner review)

- **M1, reproduction** (`tools/stage46/dial.py`, the Stage45 sandbox: an
  unprivileged namespace, private root, `/etc/hosts` and `/etc/host.conf`
  bind-mounted). A name resolving to several addresses, in four shapes:
  - `fast`: first address refused (nothing listening), second the peer;
  - `silent`: first address on a `dummy` interface route (packets dropped
    silently), second the peer;
  - `family`: `::1` first, peer on `127.0.0.1` only (today's `localhost`);
  - `all-dead`: no address answers.

  For each: direct QUIC, encrypted TCP, plain TCP target, failover QUIC
  provider, routed QUIC and TCP hop to a hostname relay, discovery and sync
  over both transports. Record echo count, time to first byte, the Stage45
  class, `sessions_failed`, the discovery status, and the receiver's quota
  during R1-style races (measured on the fix). Plus D5: the Stage42
  `fd:tcp` and `fd:quic` scenarios at `nofile` 64, recording the class.
  Expected: QUIC `fast`, `silent` and `family` fail; TCP `fast` and
  `family` pass and `silent` fails; failover QUIC refused at open; routed
  QUIC fails every session as `handshake`; D5 `handshake`.
- **M2, fix** as approved; unit tests for the dialer (order, stagger,
  window, loser closed), the classifier, and each path.
- **M3, regression:** M1 ×3; Stage45 visibility ×1 (with `localhost` now
  expected to echo); Stage44 IPv6 ×1; Stage42 floods ×1; Stage32 S2, S3,
  S4, S7 P0 (both dial paths change) and S2 P1 if T1; `ci-local` full.
- **M4:** `KNOWN_LIMITATIONS` and closure.

No wire, trust, Target Policy or receiver change.

## 7. Risks

- **Concurrent handshakes (R1).** A second QUIC or TCP attempt to the same
  peer, closed when the first wins. Bounded: at most one per address, all
  inside the existing window, and none after a winner. The receiver's
  handshake-time limits apply to each; M1 measures whether the quota is
  charged.
- **T1 changes the TCP dial on every path**, including the chaos-certified
  ones. S2, S3 and S4 rerun in M3; T0 avoids it.
- **A literal address is unchanged** under every option: one address, one
  attempt, as today. The certified lab and chaos paths all use literals,
  so they exercise the new dialer only through its single-address case.
- **Second endpoint (E1)** adds one UDP socket per Fang that dials both
  families. The Stage42 ceilings are per local connection, not per socket;
  M3's flood rerun covers descriptor counts.

## 8. Owner decisions

| ID | Decision | Options | Recommended |
|---|---|---|---|
| S46-D0 | Approve Stage46 as scoped (§6) | approve, amend, reject | approve |
| S46-D1 | When QUIC resolves a hostname | A1 at open, keep every address; A2 per session | A1 |
| S46-D2 | How addresses are tried | R1 staggered race, 250 ms; R2 sequential; R0 record only | R1 |
| S46-D3 | QUIC endpoint for the other family | E1 per family on first use; E2 dual-stack | E1 |
| S46-D4 | Which paths | P-all QUIC + T1 TCP; P-all QUIC + T0 TCP unchanged | P-all + T1 |
| S46-D5 | Local resource errors | L1 `dial` / `unreachable`; L2 new class `local`; L0 leave | L1 |
