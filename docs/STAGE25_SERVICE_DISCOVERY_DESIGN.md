# Stage25 service discovery — design proposal

**Status: design accepted 2026-10-20 (all recommendations; see
[Decisions](#decisions-accepted-2026-10-20)).** The wire extension is
specified and reviewed in step 2 with the same review as the
[Stage24 wire spec](STAGE24_FINDNODE_RPC_WIRE_SPEC.md) before any wire code
lands.

This is the next step in [ARCHITECTURE.md §65](ARCHITECTURE.md) after
distributed peer discovery: §19 ("who provides service X?") and §20
stages 3–4 (distributed service advertisements and lookup).

## Where we are

| ARCHITECTURE §20 stage | State |
|---|---|
| 1. Local static service registry | Done (Stage23: `service add/list/show/remove/connect`, `services.json`) |
| 2. Local registry + discovered peers | Done in effect: a service names a Pack peer, and Stage24 discovery finds that peer's addresses |
| 3. Distributed service advertisements | This proposal |
| 4. Distributed service lookup | This proposal |
| 5. Routing to a selected provider | Later; see non-goals |
| 6. Multi-hop service transport | Later |

Today a requester learns a service only when its operator types
`service add NAME PEER TARGET KIND`, which needs the provider's target
address out of band. The provider has no way to say "I offer `web`".

## Goals and non-goals

Discovery answers **who offers what, and where**, never **authorized?**
(§13, §19, rule 3 of §64).

- A provider's operator chooses which local services to publish.
- A requester's operator can list the services its Pack members publish and
  turn one into a local registry entry by a deliberate step.
- Pack, Pelt, Target Policy, authority, Silver and OPEN/ACK stay unchanged.
  Connecting to a discovered service goes through `service connect` and the
  normal Fang path, and the provider's Target Policy still decides.
- Out of scope: automatic provider selection, health or reputation exchange,
  service records relayed through third nodes, a Kademlia `STORE`, unknown
  (non-Pack) queriers, hostnames and multi-hop transport.

## Proposed model

### What a provider publishes

A **published service** is a provider-side entry:

```text
PublishedService {
  name   : same rules as a Stage23 service name (non-empty, canonical, <= 128 bytes)
  kind   : tcp | http | https | ssh   (a label, as in Stage23)
  target : literal IP:port on the provider, as in OPEN
}
```

at most 32 per node, persisted as `published.json` in the Den (persist
first, then publish; outside the Stage15B manifest, like `services.json`
and `contacts.json`). Publishing grants nothing: the target must still be
granted to the requester in the provider's Target Policy.

### How a requester learns it (decision 1)

| Option | Shape | Trade-off |
|---|---|---|
| **A. Ask the provider** (recommended) | A new read-only RPC op, `services`, on the Stage24 RPC; the responder returns only **its own** published services. A requester asks each Pack member directly (Pack sizes are small; bounded fan-out as `discover refresh`) | Reuses the approved, authenticated, Pack-only RPC. The answer is covered by the existing result signature, so no new self-signed record type is needed. Only nodes the requester already trusts and can reach are asked |
| B. Relay through Pack members | Providers sign standalone service records; responders relay records of their Pack members, as FindNode relays contact records | Finds services of friends-of-friends, but a relay cannot apply the provider's visibility rule (decision 2), needs a new signed record format, and the requester cannot connect to a non-Pack provider anyway |
| C. DHT `STORE` at `H(name)` | Kademlia-style storage on the nodes nearest a name hash | Nodes store others' data: an abuse, eclipse and rollback surface; contradicts Stage24 decision 2 ("a responder vouches only for its own Pack") |

### Who may see a published service (decision 2)

| Option | Rule | Trade-off |
|---|---|---|
| a. Every Pack member | Any current Pack member receives the full list | Simplest; reveals targets to peers that cannot use them |
| **b. Only peers already granted the target** (recommended) | The responder returns an entry only if its own Target Policy would accept an OPEN from this requester to that target right now (`Grants` containing it, or `LegacyAllow`; never under `Deny`/`Invalid`) | Reveals nothing a peer could not already reach; discovery *reads* authorization to limit visibility and never writes or widens it. Revoking a grant hides the entry at once |

Under (b) a requester that gets an empty answer cannot tell "no services"
from "none granted to you", which is the intended opacity.

### What the requester does with the answer (decision 3)

| Option | Shape |
|---|---|
| **Show, then import deliberately** (recommended) | `discover services` lists `(peer, name, kind, target)`; `service add` stays the operator's step. A convenience form `service add NAME --from PEER` may copy one discovered entry, but only from a fresh, verified answer, never from a cache |
| Auto-import | Discovered entries appear in the registry automatically. Not recommended: the registry would change without an operator action |

Phase 1 keeps no cache of discovered services; `discover services` always
asks.

## Wire extension (decision 4, needs review)

One new `op` value on the existing Stage24 RPC; no new `cmd`, transcript
domain or listener:

- `OP`: `"ping"`, `"find_node"` or new `"services"`.
- For `services`: `TG` is the empty string and `K` is exactly 0, as for
  `ping`.
- The result's `records` carries canonical **service entry** encodings
  instead of contact records: `u8 kind`, `LP(name)`, `LP(target)`, with the
  same strict bounded decoding as contact records. At most 32 entries; the
  result still fits `RESULT_LIMIT` (32 KiB).
- Transcripts are unchanged in shape: `OP` is already signed, so a
  `services` result cannot be presented as a `ping` or `find_node` result.
  Each entry is bound by the result signature over the retained request.

Receiver rules add one step after Stage24 step 6: for `services`, build the
list from `published.json` filtered per decision 2 for the authenticated
requester. Requester rules: `count` ≤ 32, entries decoded strictly, a
malformed entry rejects the whole result (entries are not individually
signed, unlike contact records), and nothing is ever written to the Pack,
Target Policy or registry.

Compatibility: a Stage24 receiver without `services` rejects the unknown
`op` exactly like any invalid RPC; the requester reports `rejected`, with no
fallback.

## Operator surface (draft)

```text
werewolfctl service publish NAME TARGET KIND
werewolfctl service unpublish NAME
werewolfctl service published
werewolfctl discover services [--transport quic|tcp] [--peer NAME]
werewolfctl service add NAME --from PEER          # optional, decision 3
```

Errors follow the Stage23/24 classes (`SERVICE_*`, `DISCOVER_*`,
`SILVER_ACTIVE`, `NO_PELT`). Mutations serialize with other mutations and
are refused while storage is degraded; Silver refuses `discover services`
like `discover refresh`, and a Silver-locked responder rejects the RPC.

## Security notes

- **No new authority path.** The `services` op never opens a target, takes
  an authority lease or changes state, like `ping`.
- **Visibility reads policy.** Option 2b evaluates the same Target Policy
  check OPEN uses; a bug there could reveal, but never grant. Tests must
  cover `Deny`, `Invalid`, `LegacyAllow`, missing and revoked grants.
- **Target addresses leave the provider.** Only to peers already granted
  them (2b), inside TLS, in a signed result.
- **Traffic.** One more short request/response kind inside TLS, same
  morphology note as Stage24.

## Proposed steps

| Step | Scope |
|---|---|
| 0 | This proposal; owner decisions 1–4 |
| 1 | `werewolf-core`: `PublishedService` validation, canonical entry encoding and strict decoding, unit tests (done, PR #30) |
| 2 | Wire: `services` op, receiver filter (2b), requester checks; transcript vectors and negative tests; wire contract updated ([spec](STAGE25_SERVICES_RPC_WIRE_SPEC.md), approved) |
| 3 | Operator: `service publish/unpublish/published`, `published.json`, `discover services`, optional `service add --from`; CLI contract; lab gate (provider grants one target to one of two requesters; only that requester sees it; connect works; revoking hides it) |
| 4 | Real-WAN check DarkSide ↔ VPS |

## Decisions (accepted 2026-10-20)

1. **Distribution: A.** A requester asks each Pack member directly with the
   new read-only `services` op; a responder returns only its own published
   services. No relaying, no DHT store.
2. **Visibility: b.** A responder returns an entry only if its Target Policy
   would accept an OPEN from that requester to that target at that moment
   (`Grants` containing it, or `LegacyAllow`; never under `Deny` or
   `Invalid`).
3. **Import: show, then deliberate.** `discover services` lists entries and
   changes nothing; `service add` stays the operator's step. The
   `service add NAME --from PEER` form is included and copies only from a
   fresh, verified answer, never from a cache; Phase 1 keeps no cache of
   discovered services.
4. **Wire:** the extension above (one new `op`, the service entry encoding,
   at most 32 entries within `RESULT_LIMIT`) is accepted for specification
   and review in step 2.

Step 1 notes: entry names follow the Stage23 rules and also reject control
characters, because a requester prints names received from another node;
targets must be a canonical literal `IP:port` (exactly as Rust formats a
`SocketAddr`, port non-zero, IP specified), so each target has one byte
form and checking it never resolves DNS. Kind codes are `tcp` 0, `http` 1,
`https` 2, `ssh` 3.
