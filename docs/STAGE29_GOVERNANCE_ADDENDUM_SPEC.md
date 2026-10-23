# Stage29 governance statements — addendum to the Stage28 spec

**Status: specification for review.** No code exists. This extends
[STAGE28_GOVERNANCE_STATEMENT_SPEC.md](STAGE28_GOVERNANCE_STATEMENT_SPEC.md)
with the two statement kinds of the [Stage29 design](STAGE29_ACTIVATION_DESIGN.md)
(decisions 1a–6a, additions A–C; addition D belongs to the `governance`
RPC spec of step 3). Everything not stated here is unchanged: encoding,
signing input, hash, signature rules, chain rules, forks.

## New kinds

| kind | Statement | body |
|---|---|---|
| 3 | Admission | `count:u16 count × member_public_key:[32]` |
| 4 | Activation | empty (`body_len = 0`) |

## Head additions

The head gains:

- **roster**: the admitted member keys, sorted, at most 1024 (C);
- **activated_at**: the sequence of the activation statement, if any.

Both start empty at genesis.

## Body rules

Checked after the signatures, as for Stage28 kinds; any violation rejects
the whole statement (`BodyRule`).

- **Admission (kind 3)**
  - 1–64 keys (C), in strictly increasing byte order;
  - each a valid Ed25519 point that is not of small order;
  - none on the network-revoked list (B, 2a);
  - none already on the roster;
  - the roster after admission has at most 1024 keys (C).

  Admission is valid before and after activation (B).
- **Activation (kind 4)**
  - empty body;
  - only once per chain: a second activation is rejected.
- **Network revocation (kind 2)**, as in Stage28, and in addition removes
  each revoked key from the roster.

## Local admission (addition A, as revised)

```text
Activation = network-chain fact
Admission  = local eligibility derived from that chain
```

Whether a statement is valid is the same on every node, and every node
applies a valid activation: activation is a chain state, never a node's
choice. Whether the node itself may act as a member is local:

```text
unadmitted = head.activated_at is set
             and ( own Pelt key ∉ roster  or  own Pelt key ∈ revoked )
```

It is derived from the head and the node's current Pelt, never stored, so it
changes exactly when they do: an admission of the node's key ends it, a
revocation of that key or a new Pelt (`pelt init`) begins it.

While unadmitted the node is fenced from operating as a network member:

- Fang, profile and `service connect` opens fail with
  `GOVERNANCE_UNADMITTED`, and Fangs already open are closed when the node
  becomes unadmitted;
- inbound OPENs are refused, as for a sender outside the Pack;
- it neither answers nor sends discovery and `services` RPCs;
- `governance apply` and `governance status` keep working, so the node can
  receive the admission that ends the fence, and local administration that
  only narrows trust (`pack remove`, `pack revoke`, `target remove`, Silver)
  keeps working.

`governance status` reports `admitted: true|false`. The offline tool's
`inspect --chain` does not evaluate local admission.

## Effects on the node

- **Admission:** updates the roster. It never adds a Pack member (1a).
- **Activation:** always applied when valid. In one protected generation,
  with the head, it removes every Pack member whose key is not on the
  roster or is revoked, exactly like a revocation:
  - runtime authority is denied first;
  - its Fangs and every Fang through it are closed;
  - the denial is kept if the save fails.

  It reports the removed names (3a).
- **After activation:**
  - `pack add` of a key not on the roster, or revoked, fails with
    `PACK_NOT_ON_ROSTER` or `PACK_REVOKED_BY_NETWORK` (4a);
  - at startup, an activated node whose Pack holds such a key fails closed.

## Persistence

The governance document gains `roster` (hex keys, sorted) and
`activated_at`, each omitted while empty, so a Stage28 head document keeps
exactly its bytes. A Stage28 binary refuses a document with the new fields
(strict parsing), which is the safe direction. Invariants added to the
document check:

- the roster is sorted, unique and at most 1024;
- no roster key is revoked;
- `activated_at`, if present, is at most the head sequence.

## Compatibility

A Stage28 node rejects kinds 3 and 4 (`UnknownKind`) and therefore cannot
apply any later statement in a chain that uses them. Nodes are upgraded
before admission or activation statements are delivered.

## Acceptance scenarios (owner)

Both are required tests, in core (chain) and in the lab gate (node).

**1. A node admitted after activation.**

```text
fresh node X
→ genesis
→ activation while X is not admitted
→ head advances but X remains fenced
→ later Admission(X)
→ X becomes admitted
→ no statement skipped or reordered
```

X applies every statement in sequence order; its head after activation
equals every other node's. While fenced, X's opens and inbound OPENs are
refused. The admission lifts the fence with no further statement or local
action.

**2. Revocation is permanent.**

```text
X revoked
→ later Admission(X)
→ rejected permanently
```

An admission naming a revoked key is invalid on every node (`BodyRule`).
No later statement can make it valid: there is no unrevoke, and admission
never removes a key from the revoked list. X can return only under a new
Pelt, admitted as a new key.

## Tests required before merge

- **Test vectors:** an admission and an activation from the Stage28 fixed
  seeds.
- **Negative tests:**
  - an admission of 0 or 65 keys;
  - an admission out of order;
  - an admission with a small-order key, a revoked key, or a key already on
    the roster;
  - a roster overflow at 1025;
  - an activation with a body;
  - a second activation.
- **Positive and node-local tests:**
  - a revocation removes the key from the roster;
  - an admission after activation is accepted;
  - local admission (A): an unadmitted node still advances its head on
    activation, its Fang opens and inbound OPENs are refused and its open
    Fangs closed, a later admission of its key lifts the fence, and a
    revocation of its key or a new Pelt brings it back.
- **Daemon tests:**
  - the activation sweep with Fang closure;
  - `pack add` refusals;
  - the startup check;
  - the head document round trip;
  - a Stage28 head document's bytes unchanged.

## Implementation

| Step | Scope | State |
|---|---|---|
| 1 | Core: kinds 3 and 4, roster, `activated_at`, `Head::admits`, rules, vectors (admission `6604b37e…cdf6`, activation `a4cef971…0d2f`), both owner scenarios; head document fields; offline `draft admit` and `draft activate` | Done. The daemon refuses kinds 3 and 4 (`GOVERNANCE_UNSUPPORTED`) until step 2 adds their node effects |
| 2 | Daemon: activation sweep, local admission fence, `PACK_NOT_ON_ROSTER`, startup check; lab gate | Pending |

