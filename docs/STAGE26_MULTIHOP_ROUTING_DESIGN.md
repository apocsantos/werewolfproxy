# Stage26 multi-hop routing — design proposal

**Status: design accepted 2026-10-22 (1A, 2a, 3a, 4a, 5a, 6a; see
[Decisions](#decisions-accepted-2026-10-22)).** Nothing is implemented yet.
1A needs no wire change: step 1 must prove the encrypted-TCP transcripts
and framing are byte-identical before any routing code lands.

This is the next step in [ARCHITECTURE.md §65](ARCHITECTURE.md) after
distributed service discovery: §21 (multi-hop routing), §22 (end-to-end
versus hop-by-hop trust), §49 (routing policy) and §20 stage 6 (multi-hop
service transport).

## Where we are

| ARCHITECTURE §20 stage | State |
|---|---|
| 1–2. Local registry, discovered peers | Done (Stage23, Stage24) |
| 3–4. Distributed service advertisements and lookup | Done ([Stage25](STAGE25_SERVICE_DISCOVERY_DESIGN.md)) |
| 5. Routing to a selected provider | Not started; see decision 6 |
| 6. Multi-hop service transport | This proposal |

Today a Fang reaches a peer only by dialling that peer's Pack address
directly. If A cannot reach D (D behind NAT, D listening only on a private
network, A's network blocking D's port), A cannot use D's services at all,
even when a Pack member B can reach both. The Stage22R and Stage25 WAN
checks hit exactly this: the VPS cannot reach DarkSide, so nothing behind
DarkSide is usable from anywhere else.

## Goals and non-goals

- A reaches D's service through one intermediate Pack member B (the
  **relay**), with **end-to-end** identity and authorization: A pins D's
  key, D verifies A's signed OPEN, and D's Target Policy decides, exactly as
  on a direct Fang (§22: routing intermediaries ≠ application authority).
- The relay forwards only because its own operator allowed it, deny by
  default, and learns no more than it must (§21).
- Pack, Pelt, Target Policy semantics, authority, Silver, OPEN/ACK, the
  Stage10 transcripts, KDFs and framing stay unchanged at A and at D.
- Out of scope: automatic route discovery or selection, routes learned from
  other nodes, more than one relay, QUIC end-to-end inside a relay,
  discovery RPCs (`ping`, `find_node`, `services`) through a relay,
  anonymity or traffic-analysis resistance, and provider selection
  (decision 6).

## Proposed model

### What a route is

A **route** is a requester-side entry saying "reach Pack member D through
Pack member B":

```text
Route {
  peer  : a Pack member name (the destination D)
  via   : another Pack member name (the relay B)
  hop   : quic | tcp      transport from A to B
}
```

D's Pack record is unchanged: its key is still the end-to-end trust root,
and its `tcp://` address is where B must connect. A route never changes
who D is; it only changes how the bytes get there.

### How the bytes travel (decision 1)

**1A. Nested session through an ordinary target (recommended).**
A opens an ordinary Fang session to B whose OPEN target is D's encrypted-TCP
listener (`IP:port` from D's Pack address). B authorizes that exactly like
any other target: its Target Policy must grant A that `IP:port`. Over the
resulting byte stream A then runs the unchanged encrypted-TCP client
protocol to D: TLS pinned to D's key, D's challenge, A's signed OPEN for the
real target, ACK, framed AEAD data.

```text
A ──[outer: QUIC or encrypted TCP, A↔B keys]── B ──[plain TCP]── D:8443
   └──────────[inner: encrypted TCP, TLS pinned to D, A↔D keys]──────────┘
```

- No new wire format anywhere. B runs the code it runs today; D sees an
  ordinary encrypted-TCP session arriving from B's address.
- The A-side change is that the encrypted-TCP client must run over any
  authenticated byte stream instead of only a `TcpStream` (its copy loop is
  already generic), with the TLS server name taken from D's Pack address
  instead of the socket's peer address.
- Weakness against §49: "may route through B" is expressed as a Target
  Policy grant on B for D's listener address. It is explicit and deny by
  default, but B cannot tell that the target is a Werewolf listener, and
  routing and service grants share one list.

**1B. A dedicated relay operation.** A new OPEN variant asks B to connect
to *Pack member D* by name or key; B checks a separate **relay policy**
("A may route through me to D") and dials D's Pack address itself. Cleaner
planes (§49), and B can refuse anything that is not one of its Pack members.
Costs a wire extension, a new persisted policy document and a full wire
review; the inner session is the same as in 1A.

**1C. Hop-by-hop re-origination.** B opens its own Fang to D as B. Rejected:
D would authorize B, not A, turning the network into a chain of trusted
proxies (§22).

### Where routes live (decision 2)

**2a. `routes.json` in the Den (recommended)**, persisted first then
published, outside the Stage15B manifest like `services.json`,
`published.json` and `contacts.json`. A route grants nothing: a rolled-back
or tampered route can at worst send traffic to a different relay of A's
own Pack, where it still needs that relay's grant, and D still
authenticates end to end. **2b.** A new field in the Pack record, inside
the protected manifest: rollback-protected, but a manifest schema change.

### How far (decision 3)

**3a. Exactly one relay (recommended).** **3b.** Chains of relays by
nesting 1A recursively. The mechanism allows 3b later; each extra relay adds
one layer of encryption, framing and latency, and one more operator who
sees the session exists.

### Which transports (decision 4)

**4a (recommended).** The outer hop A→B is QUIC or encrypted TCP, per
route; the inner A↔D session is always encrypted TCP, because the relay
carries a byte stream and QUIC needs datagrams. Plain TCP is never used
inside a relay. **4b.** Also carry QUIC end to end (UDP relaying): a new
transport, out of scope here.

### What may use a route (decision 5)

**5a (recommended).** Fang open and `service connect` only. **5b.** Also
the Stage24/25 RPCs, so `discover services` and `service add --from` work
through a relay; a follow-up once 5a is certified.

### Provider selection (decision 6)

§20 stage 5 (choose among several providers of one service) is independent
of multi-hop. **6a (recommended).** Keep it out of Stage26 and propose it
separately. **6b.** Fold a manual form into Stage26.

## Operator surface (draft, for 1A + 2a)

```text
werewolfctl route set PEER --via RELAY [--hop quic|tcp]
werewolfctl route list
werewolfctl route remove PEER
```

`fang` and `service connect` to PEER then go through RELAY automatically;
`fang active` shows the relay. On the relay, the operator grants as today:

```text
werewolfctl target allow A D_IP:D_TCP_PORT
```

Errors follow the existing classes (`ROUTE_INVALID`, `ROUTE_NOT_FOUND`,
`ROUTE_SAVE_FAILED`, plus the Fang errors of whichever hop failed). Route
mutations serialize with other mutations and are refused while storage is
degraded; Silver refuses a routed Fang like any Fang.

## Security notes

| Who | Learns | Does not learn |
|---|---|---|
| Relay B | A's identity (A authenticates to B); D's listener address; when and how much A sends | The service, the target on D, the payload, A's OPEN to D (all inside TLS pinned to D) |
| Destination D | A's identity and target, as today; B's IP address | A's IP address |
| A | Nothing new | — |

- **No new authority path.** B forwards only under its existing Target
  Policy (1A) or a new deny-by-default relay policy (1B); D authorizes A
  end to end. A relay can deny, delay or drop, never impersonate D or
  widen A's access on D.
- **Each hop keeps its own controls.** Stage16 bounds apply independently
  at B (one outer session, one target connect) and at D (one inner
  session). Revoking A on B, removing B's grant or Silver on B cuts the
  outer session and therefore the inner one; revoking A on D cuts the inner
  one; Silver on A stops both.
- **Session end.** Inner session-end signalling is unchanged; an outer
  failure reaches the application as a failure of the inner session. D06
  and D07 equivalents must be re-run through a relay before this is
  claimed.
- **Cost.** Double encryption and framing, and the inner session's 2 KiB
  frames inside outer frames; throughput and latency are measured, not
  claimed.
- **Traffic.** B sees the outer session's timing and volume, as any
  network observer between A and B already does today.

## Proposed steps

| Step | Scope |
|---|---|
| 0 | This proposal; owner decisions 1–6 |
| 1 | Refactor: the encrypted-TCP client runs over any authenticated byte stream (done, PR #38: the client handshake moved verbatim into `client_handshake`, same `hs::` calls and order; tests drive it through an in-memory pipe against the unmodified receiver; no wire change) |
| 2 | Routes: `routes.json`, `route set/list/remove`, CLI contract, unit tests (done, [CLI contract](V1_CLI_CONTRACT.md#stage26-extension-routes)) |
| 3 | Routed Fang and `service connect`; lab gate with three wolves (done, `tests/integration/routed_fang_gate.py`; see step 3 notes) |
| 4 | Real-WAN check: DarkSide → VPS (relay) → a destination reachable only from the VPS (a second daemon on the VPS's loopback, or a third host), plus D06/D07-style failure checks through the relay |

## Decisions (accepted 2026-10-22)

1. **Mechanism: 1A.** A nests an unchanged encrypted-TCP session to D inside
   an ordinary Fang session to B, whose Target Policy grants A D's listener
   address. No wire change; 1B (relay op) stays a possible later
   refinement, 1C is rejected.
2. **Routes: 2a.** `routes.json` in the Den, persist first then publish,
   outside the Stage15B manifest.
3. **Reach: 3a.** Exactly one relay.
4. **Transports: 4a.** Outer hop QUIC or encrypted TCP per route; inner
   session always encrypted TCP; never plain TCP inside a relay.
5. **Scope: 5a.** Fang open and `service connect` only; discovery RPCs
   through a relay are a follow-up.
6. **Provider selection: 6a.** A separate stage.

Step 2 notes: `route set` requires both PEER and RELAY to be Pack members at
that moment (use time checks again in step 3) and replaces PEER's existing
route; a route whose relay is itself routed, or whose peer relays for
another route, is refused, so the list always describes exactly one relay
per destination. At most 256 routes. Routes are not part of the protected
state generation.

Step 3 notes: the routed forwarder (`transport/relay.rs`) opens, per local
connection, the outer session with the existing client code (QUIC
`connect_v3_on`, or `connect_authenticated_tcp` plus `client_handshake`) and
runs the step 1 client over it. Over a QUIC hop the inner session runs on the
joined QUIC stream pair and, as for a direct QUIC Fang, a clean end waits for
the relay to acknowledge the whole upload; over a TCP hop the existing framed
copy loop feeds one end of an in-memory pipe that carries the inner session.
Any failure at either hop resets the local connection. The lab gate cannot
make C unreachable from A on loopback, so instead of "A cannot dial C" it
proves the path through the grants: either grant alone fails, and B and C
each hold one authority session while a routed session is open. Revoking a
grant on a live session was not added to the gate; Silver on the relay was.
Session-end behaviour through a relay (D06/D07 equivalents) is not yet
claimed; it belongs to step 4.

