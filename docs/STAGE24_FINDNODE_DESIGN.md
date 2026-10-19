# Stage24 FindNode RPC — design proposal

**Status: design accepted 2026-10-18; Phase 1 implemented.** The RPC
extends the [v1 wire contract](V1_WIRE_CONTRACT.md) as described in the
[wire spec](STAGE24_FINDNODE_RPC_WIRE_SPEC.md).

| Step | Scope | State |
|---|---|---|
| 0 | Bounded k-buckets, XOR ordering, `k` cap (`werewolf-core`) | Done (PR #8) |
| 1 | Pelt-derived `NodeId`, signed `ContactRecord` (`werewolf-core::contact`) | Done |
| 2 | FindNode/Ping RPC on the OPEN path, Pack-only ([wire spec](STAGE24_FINDNODE_RPC_WIRE_SPEC.md)) | Done |
| 3 | Advertised own record, lookup, `contacts.json` cache, `werewolfctl discover`, three-wolf lab gate | Done ([CLI contract](V1_CLI_CONTRACT.md#stage24-extension-discovery)) |

Next item in [ARCHITECTURE.md §65](ARCHITECTURE.md): run FindNode over
authenticated Werewolf transport (§18), as the base for distributed peer
discovery (§15) and later service discovery (§19–20).

## Current state

`werewolf-core` has Kademlia-shaped types (`nodeid`, `peer`, `kbucket`,
`routing`, `lookup`, `find_node`, `message`, `peer_store`) used only by unit
tests and the `peer_lab` binary. `werewolfd` and `werewolfctl` never reference
them. They are not safe to put on the wire as they are:

| Issue | Where | Consequence |
|---|---|---|
| `NodeId = SHA-256(name "\|" address)` | `nodeid.rs` | Not bound to a Pelt; anyone can claim any ID, and the ID changes when an address changes |
| `Peer` carries self-reported `healthy`, `reputation`, `availability_pct`, `latency_ms` (f64) | `peer.rs` | A remote node could inject its own reputation; local measurements would leak |
| `FindNode.k: usize` is unbounded | `message.rs` | A request can ask for the whole table |
| `KBucket` has no capacity or eviction | `kbucket.rs` | Unbounded memory; no Kademlia churn resistance |
| `nearest()` sorts by bucket index, then reputation | `routing.rs` | Not true XOR order; lookups need not converge |
| Results are unsigned `(name, address)` pairs | `find_node.rs` | A responder can forge addresses for other nodes |

## Goals and non-goals

FindNode answers **who and where**, never **authorized?** (§19).

- Returns candidate contact records close to a target ID.
- Never adds a node to the Pack, never creates a target grant and never opens
  a Fang. A discovered node is used only after the operator runs `pack add`.
- Pack, Target Policy, Silver and the existing OPEN/ACK semantics stay
  unchanged.
- Out of scope: service advertisements, multi-hop routing, NAT traversal,
  reputation exchange and an open public overlay.

## Proposed model

### Node identity

`NodeId = SHA-256("werewolf/node-id/v1" || Ed25519 public key)`. Pack names
stay local labels. Changing an address no longer changes the ID, and an ID
cannot be claimed without its Pelt.

### Signed contact records

Each node signs its own record:

```text
ContactRecord {
  public_key   : Ed25519 public key (32 bytes)
  addresses    : 1..=4 transport URLs (quic:// or tcp://), each <= 256 bytes
  seq          : u64, strictly increasing per node
  expires_at   : unix seconds, at most 7 days after signing
  signature    : Ed25519 over domain "werewolf/contact/v1" + canonical encoding
}
```

Responders relay records unchanged and cannot forge them. They can still
withhold records or return stale ones (eclipse), so the lookup prefers higher
`seq` and queries several peers. Health, latency and reputation stay local and
are never sent.

### Messages

```text
FindNode       { target: NodeId, k: u8 (1..=20) }
FindNodeResult { records: Vec<ContactRecord> (<= k, <= 16 KiB encoded) }
Ping / Pong    { }   // liveness only, for bucket eviction
```

The sender's identity comes from the authenticated session, never from a
message field, so the current `sender: NodeId` fields go away.

### Transport (decision needed)

| Option | Shape | Trade-off |
|---|---|---|
| **A. Extend OPEN** (recommended) | A new signed request kind next to the target OPEN on the same QUIC/encrypted-TCP listener, reusing TLS 1.3 exact-SPKI receiver auth and the exporter/challenge-bound sender signature, with its own domain tag `werewolf/rpc/v1` | Reuses audited authentication; no new listener or port to fingerprint. It is a wire-contract extension and needs the same review as Stage10/12 |
| B. Separate RPC listener | Own port, same TLS/SPKI rules, its own sender proof | Leaves the frozen OPEN path untouched, but adds a second authenticated handshake to certify and a new observable port |
| C. Inside an open Fang | RPC framed over an existing Fang stream | Mixes control traffic with application authority; not recommended |

Under A, a receiver that does not implement the new kind must fail exactly as
it does for any invalid OPEN today, with no capability hint (wire opacity).

### Who may query

Phase 1 (recommended): **closed overlay**. Only authenticated Pack members may
send FindNode, and only records of nodes the responder has itself verified are
returned. That matches deny-by-default and gives a useful "friend-of-friend"
lookup without an open DHT. An open overlay (unknown queriers, Sybil
resistance) is a later decision.

### Routing table fixes

- k = 20 per bucket, insertion order kept, ping-before-evict on a full bucket.
- `nearest()` sorts by full 256-bit XOR distance.
- Lookup: α = 3 parallel queries, at most 8 rounds, per-query deadline from the
  existing handshake limits, stop when the k closest have answered.
- Per-peer rate limit and a global concurrency cap through the Stage16
  pre-auth limits.

### Persistence

Discovered records go to a non-authoritative cache (`contacts.json` in the Den,
private like `services.json`, outside the Stage15B manifest). Losing or rolling
back the cache changes only lookup hints, never authority. The Pack stays the
only trust source.

### Operator surface (draft)

```text
werewolfctl discover find PUBLIC_KEY_B64 | NODE_ID_HEX
werewolfctl discover contacts
```

`discover find` prints the verified record. Trusting the result is still a
deliberate, separate `pack add`.

## Test plan

- Unit: record signature and domain separation, canonical encoding, `seq`
  ordering, expiry, every size bound, XOR ordering, bucket eviction.
- Hermetic lab (`tests/integration`, three wolves): A knows B and B knows C;
  A finds C's verified record; A still cannot open a Fang to C until
  `pack add`, and B's Target Policy still governs targets on B.
- Negative: forged or expired record, oversized result, `k` out of range,
  non-Pack querier, querier under Silver, replayed RPC OPEN, unknown request
  kind on a receiver without support.

## Decisions (accepted 2026-10-18)

1. Transport: **A**, a new signed request kind on the existing OPEN path.
2. Query admission: **Pack-only** closed overlay for Phase 1.
3. `NodeId` is derived from the Pelt public key and stable across addresses,
   accepting that a node is linkable inside the overlay.
4. Records live at most **7 days** (checked against the verifier's clock) and
   are re-signed with a higher `seq` on address change.
5. The current `werewolf-core` Kademlia types may be replaced; only
   `peer_lab` uses them.

Step 1 notes: records use a canonical binary encoding (key, length-prefixed
addresses, `seq`, `expires_at`, signature), strict bounded decoding and
Ed25519 `verify_strict`. Addresses must be literal `IP:port` with a
`quic://` or `tcp://` scheme; hostnames are rejected so discovery never
triggers DNS. Whether loopback/private addresses learned from other nodes
should be ignored is left to step 3.

## Step 3 decisions (2026-10-19)

These narrow the proposal above for Phase 1; none changes the wire messages.

1. **Lookup is one hop.** `discover find` queries only Pack members (the
   eight nearest to the target, α = 3, one seven-second budget) instead of
   iterating over discovered nodes. A discovered node is shown to the
   operator and never contacted automatically, so a Pack member cannot steer
   this node into connecting to arbitrary addresses. This also settles the
   loopback/private-address question: no filter is needed while learned
   addresses are never dialled. Multi-hop lookup is a later decision.
2. **A responder vouches only for its own Pack.** The "verified contact set"
   a FindNode answer draws from is the cached records whose key belongs to a
   current Pack member. Records learned from other nodes are cached for the
   operator but never relayed, and removing a member stops relaying it at
   once.
3. **Own record.** The operator sets the addresses (`discover advertise`,
   persisted as `advertise.json`); nothing is inferred from listener binds.
   The record is signed in memory with `seq` = max(clock, previous + 1) and
   re-signed when addresses change or less than a day of validity is left,
   so `seq` needs no persistence. A clock set backwards can make peers keep
   an older record until it expires.
4. **Operator-driven traffic.** `discover refresh` (ping every Pack member)
   and `discover find` run only on request; there is no background refresh
   or bucket maintenance, so discovery adds no unsolicited traffic.
5. **Cache.** `contacts.json` holds at most 256 verified records, the highest
   `seq` per node, evicting non-Pack records and the earliest expiry first.
   It is written before it is published, outside the Stage15B manifest; a
   malformed cache is ignored at startup, while Den errors still fail
   closed.
6. **Silver** refuses refresh and find, like a Fang open.
