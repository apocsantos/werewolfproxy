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

## Node-local rule (addition A)

Whether a statement is valid is the same on every node. Applying an
activation additionally requires that the **applying node's own current
Pelt public key** is on the roster and not revoked. A node that fails this
refuses the activation (`GOVERNANCE_NOT_ADMITTED`), keeps its head
unchanged, and stays before activation. The check is the daemon's, not the
chain's: the offline tool's `inspect --chain` does not apply it.

## Effects on the node

- **Admission:** updates the roster. It never adds a Pack member (1a).
- **Activation:** in one protected generation, with the head, it removes
  every Pack member whose key is not on the roster or is revoked, exactly
  like a revocation:
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
  - the node-local refusal (A).
- **Daemon tests:**
  - the activation sweep with Fang closure;
  - `pack add` refusals;
  - the startup check;
  - the head document round trip;
  - a Stage28 head document's bytes unchanged.
