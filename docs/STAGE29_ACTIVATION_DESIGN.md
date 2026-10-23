# Stage29 admission and activation — design proposal

**Status: proposal for architectural review, awaiting owner decisions 1–6.**
Nothing here is implemented. Like Stage28, it changes the trust model, so it
is reviewed before any code.

Stage28 built the governance chain: 7-of-10 statements for genesis, key
rotation and network revocation, verified by every node and stored in its
protected manifest ([survey](STAGE28_GOVERNANCE_SURVEY.md),
[spec](STAGE28_GOVERNANCE_STATEMENT_SPEC.md)). Stage29 takes the next part
of ARCHITECTURE §27–30: a **network roster** of admitted members, an
**activation** statement that ends bootstrap, and what local administration
may still do afterwards (survey decision 6b). It also proposes how
statements reach nodes without an operator copying files (survey 4b).

## What governance can and cannot enforce

This shapes every decision below, so it comes first.

- **On other hosts, yes.** If node X refuses Pack membership to any key not
  on the roster, a compromised host Y cannot make X trust a stranger, and a
  revoked member is refused everywhere the revocation has been applied.
  This is the network-level guarantee: one host, one operator or one key
  cannot widen trust anywhere but on itself.
- **On its own host, no.** OS root on Y can rewrite Y's Den: the protected
  manifest detects partial or mixed tampering, but a complete, consistent
  rewrite (recomputing the commitments, or writing a schema-1 manifest with
  no governance) is possible for root. Y can then trust anyone *itself*.
  Activation cannot prevent that; it makes it pointless, because nobody
  else will accept the strangers Y trusts.

So "root becomes unobtainium" (§27) is achieved for the **network's**
authority, not for an individual host's files. The proposal says so
plainly rather than implying more.

## Proposed model

### The roster (decisions 1 and 2)

Two new statement kinds in the Stage28 chain, same encoding and verification
(at least 7 of 10 signatures, next in chain):

| kind | Statement | body |
|---|---|---|
| 3 | Admission | `count:u16 count × member_public_key:[32]`, strictly increasing |
| 4 | Activation | empty |

The head gains a sorted **roster** of admitted member keys. Admission adds
keys; network revocation (kind 2) also removes a key from the roster.

**1a. The roster is an upper bound (recommended).** After activation a node
may only have roster members in its Pack; it still chooses which of them,
under which names and addresses. **1b. The roster is authoritative**: every
node adds every roster member automatically. Rejected: it widens each
node's trust without its operator's consent, and Pack entries need local
names and addresses anyway.

**2a. Revocation is permanent (recommended).** A revoked key can never be
admitted again; a member who needs back in creates a new Pelt and is
admitted under it. **2b.** Allow re-admission by an explicit statement:
more flexible, but it turns revocation into a toggle that seven keys can
flip back.

### Activation (decision 3)

Activation is one-way and applies from its sequence on. Before it, a node
behaves exactly as in Stage28. After it, the lockdown of decision 4
applies. Applying activation on a node whose Pack contains keys that are
not on the roster:

**3a. Removes them, as a revocation would (recommended)**: authority denied
first, their Fangs closed, the Pack and head committed together, and the
removed names reported. It always moves toward less trust, and the node
stays able to apply every later statement, including future revocations.
**3b. Refuses activation** until the operator removes them. Gentler, but a
node stuck before activation can never apply a later revocation, which is
the worse failure.

### The lockdown (decision 4)

**4a (recommended).** After activation:

| Operation | After activation |
|---|---|
| `pack add` | Only roster keys that are not revoked (`PACK_NOT_ON_ROSTER`) |
| `pack set-address`, `pack remove`, `pack revoke` | Unchanged (they never widen trust) |
| Target grants | Unchanged: they already require a Pack member, so they are bounded by the roster |
| Silver on/off | Unchanged: an emergency control that must work without a ceremony |
| `pelt init` on an existing node | Unchanged: a new Pelt is simply not on the roster, so other nodes refuse it |
| Governance | A node cannot leave or reset its network |

At startup an activated node also checks its Pack against the roster and
fails closed if a non-roster key is present, so a tampered Pack file is
caught even when the manifest is consistent.

**4b.** Additionally require statements for target grants and Silver reset.
It locks more, but it puts routine service configuration and emergency
recovery behind a seven-person ceremony, which the architecture reserves for
"rare high-authority lifecycle operations" (§25).

### Delivery between nodes (decision 5)

**5a. Pull on demand (recommended).** A new read-only RPC op `governance`
on the Stage24 channel: a Pack member asks for statements after its
sequence and the responder returns the next ones from its chain (bounded
count and size, signed result as for `find_node`). `werewolfctl governance
sync` asks Pack members and applies what verifies, through exactly the same
`apply` path as a file. Responders gain nothing: a statement is valid only
with its seven signatures, so a peer can withhold statements but never
forge one. Needs its own wire spec and review, as Stage24 and Stage25 did.
**5b.** Stay operator-delivered only. **5c.** Pull automatically on a
timer: faster revocation, but a background network parser; better as a
follow-up once 5a is proven.

### Format (decision 6)

**6a (recommended).** Admission and activation are kinds 3 and 4 of the
Stage28 encoding, added as an addendum to the Stage28 spec with test
vectors; the head document gains `roster` and `activated_at`. A Stage28
node refuses kinds 3 and 4 (`UnknownKind`), so a chain that uses them
needs upgraded nodes before those statements are delivered.

## Proposed steps

| Step | Scope |
|---|---|
| 0 | This proposal; owner decisions 1–6 |
| 1 | Spec addendum (kinds 3 and 4, roster, activation rules) for review, then core: encoding, verification, vectors and negative tests |
| 2 | Daemon: roster and activation in the head, the activation sweep (3a), `PACK_NOT_ON_ROSTER`, the startup check; lab gate |
| 3 | `governance` RPC op: wire spec and review, then `governance sync`; lab gate with a revocation reaching a node through a peer |
| 4 | A real-host check on disposable daemons only (genesis is irreversible) |

## Decisions

Pending. Recommended: **1a, 2a, 3a, 4a, 5a, 6a.**
