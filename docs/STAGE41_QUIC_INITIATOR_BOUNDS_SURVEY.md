# Stage41 — QUIC initiator resource-bound parity (survey)

**Status: SURVEY, documentation only.** It proposes no code until the
owner decides §8. No production source, harness, dependency or bound
changes in this PR.

Source: Stage39 survey, finding S2 and §13 recommendation 2.

## 1. Baseline

- `main` `6e72d26` (Stage40 merged).
- quinn-proto 0.11.18, quinn 0.11.x, tokio 1.52.3. On 2026-11-09 the
  latest quinn-proto release is still 0.11.19.
- S3-P2 stays EXCLUDED (Stage34). `v1.0.0-rc.2` is frozen.

## 2. Where the initiator's QUIC configuration lives

One function builds every outbound QUIC endpoint:
[`client_endpoint`](../crates/werewolfd/src/quic_fang.rs) (`quic_fang.rs:128–145`).
It starts from `quinn::TransportConfig::default()` and sets only
`keep_alive_interval` (10 s).

Its callers:

| Caller | Purpose |
|---|---|
| `quic_fang.rs:34` | local QUIC Fang (one connection per accepted local TCP connection) |
| `transport/failover.rs:142` | secure fallback, QUIC leg |
| `transport/relay.rs:122`, `:287` | routed Fang, QUIC hop to the relay |
| `discovery.rs:476` | discovery queries |

So a change to `client_endpoint` covers direct, failover, routed and
discovery traffic at once. The lab tools (`quic_lab.rs`, `bin/quic_*`)
use a separate endpoint and are out of scope.

## 3. How the initiator uses a connection (from the code)

- **One stream per connection.** `connect_v3_on` opens a single
  bidirectional stream (`quic_fang.rs:186`). The v3 handshake and then
  the session's data both run on it. Discovery does the same
  (`discovery.rs:493`).
- **The initiator never accepts a stream.** There is no `accept_bi` or
  `accept_uni` anywhere on the client side.
- **No QUIC datagrams.** Nothing sends or reads them.
- **Who decides how many connections exist:** local clients do. Each
  local TCP connection to a Fang makes one QUIC connection. There is no
  separate ceiling on Fang connections (outside this stage's scope;
  see §7).

## 4. Receiver, initiator and quinn defaults side by side

| Parameter | Receiver (`transport/quic.rs:24–62`) | Initiator today (quinn 0.11.18 default) | What the remote peer controls on the initiator |
|---|---|---|---|
| Peer-opened bidi streams | 64 | **100** | 100 streams the initiator never reads |
| Peer-opened uni streams | 0 | **100** | 100 streams the initiator never reads |
| Stream receive window | 64 KiB | **1.25 MB** (1 250 000 B) | per stream |
| Connection receive window | 512 KiB | **unlimited** (`VarInt::MAX`) | the sum over all streams |
| Send window | 512 KiB | **10 MB** (8 × 1.25 MB) | how long the peer delays ACKs keeps it full |
| Crypto buffer | 16 KiB | 16 KiB | equal |
| Datagram receive buffer | none (disabled) | **1.25 MB** | datagrams that are never read |
| Idle timeout | 30 s | 30 s | equal (the lower one applies) |
| Keep-alive | — | 10 s | — |
| Incoming connection limits | 64, 16 KiB, 1 MiB | n/a (client endpoint) | — |

### 4.1 Worst case per initiator connection (arithmetic, not measured)

A malicious peer that passes exact-SPKI pinning can open 100 bidi and 100
uni streams and fill each stream window. The connection window does not
stop it, because it is unlimited. That is up to **200 × 1.25 MB ≈ 250 MB**
buffered per connection, plus 1.25 MB of datagrams. Nothing ever reads
it, so it stays until the connection closes. The idle timeout does not
apply, because the peer is sending.

The receiver bounds the same thing at 512 KiB per connection.

**Evidence levels:** the defaults and call sites are CONFIRMED from code.
The 250 MB figure is an upper bound worked out from quinn's
configuration. It is NOT measured: quinn may allocate lazily, so the real
peak depends on what the peer actually sends.

### 4.2 Who can exploit it

Only a peer the initiator chose and authenticated: its Pack key is
pinned before any 1-RTT data. In practice that means a compromised or
malicious Pack member, or a relay. It is not reachable by an
unauthenticated host. This matches the Stage39 rating: **MEDIUM-LOW.**

## 5. Two kinds of change

The parameters fall into two groups, and they carry different risk.

### 5.1 Unused surface (no effect on the one stream that carries data)

- peer-opened bidi streams → **0**
- peer-opened uni streams → **0**
- datagram receive buffer → **disabled**
- connection receive window → **bounded**, at or above the stream
  receive window (with one stream, a connection window ≥ stream window
  changes nothing on the data path)

These close §4.1 completely: with no peer-opened streams, the peer can
fill only the one stream the initiator reads. They do not change any
window on the data path, so they should not change throughput.

**Check needed:** quinn must accept 0 peer-opened bidi streams on a
client whose own streams are locally opened. Per RFC 9000 the limit
applies only to streams the *peer* opens. Step 2 unit tests confirm
this, including that the v3 handshake still works.

### 5.2 Data-path windows (change throughput)

- stream receive window (download direction: receiver → initiator)
- send window (upload direction: initiator → receiver)

Lowering these to the receiver's values (64 KiB, 512 KiB) would bring
full symmetry, but:

- they cap throughput on high-latency links (64 KiB per RTT ≈ 640 KB/s
  at 100 ms);
- the **send window is the Stage34 regression surface.** The leading
  hypothesis for the routed-QUIC regression is send-buffer accounting in
  quinn-proto 0.11.18, on the A→Y hop, where A is this initiator. Changing
  A's send window from 10 MB to 512 KiB could change S3 cell behaviour
  either way. Stage34 forbids changing a window to make S3-P2 pass, and
  equally a window change must not be judged by S3-P2.

The download direction is already limited by the receiver's 512 KiB send
window, so lowering the initiator's stream window below that only
narrows the pipe. The local client owns upload memory: it supplies the
data, and the 10 MB send window is a local cost per connection, not a
remote-controlled one.

## 6. Options

| Option | Changes | Closes §4.1 | Throughput risk | Touches the Stage34 surface |
|---|---|---|---|---|
| **A. Unused surface only** | §5.1 | yes | none expected | no |
| **B. A + matching receive windows** | §5.1, plus stream receive window 64 KiB and connection window 512 KiB | yes | download narrowed | indirectly |
| **C. Full parity** | B, plus send window 512 KiB | yes | both directions | **yes** (send window) |
| **D. Do nothing** | — | no | — | — |

**Recommendation: A**, with the connection receive window set to the
quinn default stream window (1.25 MB), so the data path is unchanged.
A remote peer then controls at most ~1.25 MB per initiator connection
instead of ~250 MB. B and C are throughput decisions, not security fixes,
and C also touches the Stage34 surface. They should be a separate,
measured decision if ever wanted.

## 7. Related findings (out of scope, recorded only)

- **No ceiling on Fang connections.** Initiator memory scales with the
  number of local connections. Local clients are trusted, so this is not
  a remote attack, but it is unbounded. A possible later stage.
- **The initiator binds `0.0.0.0:0`** (IPv4 only): the IPv6 stage.
- **Bounds in two places.** The receiver constants live in
  `transport/quic.rs`. Option A would add initiator constants in
  `quic_fang.rs`, next to the client config, with a comment linking the
  two.

## 8. Proposed Stage41 contract (for owner review)

- **M1, tests first:** a unit test, in the style of
  `transport/quic/resource_tests.rs`, that builds `client_endpoint`'s
  transport config against a test server and shows that on `main` the
  server can open uni and bidi streams toward the initiator and send
  datagrams. It FAILS on `main` by design.
- **M2, fix:** Option A in `client_endpoint` only. The M1 test passes.
  The workspace tests, fmt, clippy and `scripts/ci-local.sh` (full)
  pass.
- **M3, regression:** the Stage32 cells that use `client_endpoint`, 3
  runs each, with the same rules as Stage39 M3: S2 (QUIC direct) at P0,
  P1, P2 and P5; S3 (routed, QUIC hop) at P0 and P1; S4 (establishment)
  at P0 and P2; S5 (discovery) at P0; S7 (selection, QUIC impaired) at
  P0. S2-P4 stays EXCLUDED (F3). **S3-P2 is not a criterion** and stays EXCLUDED; if it is
  run at all, the result is recorded only. Every cell must keep its
  historical verdict.
- **M4:** a docs-only record and the `KNOWN_LIMITATIONS` update.

Stop conditions: if M1 cannot show the exposure, Stage41 stops at M1 and
records why. If M3 shows any changed verdict, the owner decides before
merge.

## 9. Owner decisions

| # | Decision | Options | Recommended |
|---|---|---|---|
| S41-D0 | Approve Stage41 as scoped (§8) | approve, amend, reject | approve |
| S41-D1 | Which option | A, B, C, D (§6) | **A** |
| S41-D2 | Connection receive window value under A | 1.25 MB (quinn's stream window; data path unchanged); 512 KiB (receiver's value; may narrow downloads); other | 1.25 MB |
| S41-D3 | Discovery shares the same config | yes (one function); no (separate config) | yes |
| S41-D4 | M3 regression scope | the §8 subset; the full network matrix | the §8 subset (every cell whose initiator uses `client_endpoint`) |
| S41-D5 | Fang connection ceiling (§7) | later stage; never; include now | later stage, separate survey |

**Not authorised by this survey:** implementing Stage41, any window or
timeout change on the data path, any change judged by S3-P2, tokio
promotion, rc.3, release certification.
