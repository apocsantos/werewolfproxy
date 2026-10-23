# Stage28 governance statements — format and verification spec

**Status: specification approved 2026-10-23 (S1–S4 as recommended; see
[Decisions](#decisions-for-this-spec)).** No code exists yet. This implements decisions 1a, 2a, 3a, 4a, 5a, 6a and 7a of
the [governance survey](STAGE28_GOVERNANCE_SURVEY.md) and is reviewed like
the [Stage24 wire spec](STAGE24_FINDNODE_RPC_WIRE_SPEC.md) before any code
lands. It defines what a governance statement is, exactly which bytes are
signed and hashed, how a node verifies and applies one, and how it records
the result.

## Terms

- **Governance key:** an Ed25519 key held offline by one key holder. Nodes
  know only the 32-byte public keys, by index 0–9.
- **Statement:** a signed record of one governance action.
- **Chain:** statements in sequence order from genesis; each names the hash
  of the one before it.
- **Head:** the last statement a node applied: network ID, sequence, hash,
  the current key set and threshold, and the network-revoked list.

## Encoding

All integers are big-endian; there are no optional fields, no padding and no
trailing bytes. A statement is at most 16 KiB.

```text
statement   = core signatures
core        = version:u8 (=1)
              network_id:[32]
              sequence:u64
              previous:[32]
              kind:u8
              body_len:u16  body:[body_len]
signatures  = count:u8  count × ( key_index:u8  signature:[64] )
```

| kind | Statement | body |
|---|---|---|
| 0 | Genesis | `threshold:u8 key_count:u8 key_count × public_key:[32]` |
| 1 | Key rotation | `count:u8 count × ( key_index:u8 public_key:[32] )` |
| 2 | Network revocation | `count:u16 count × member_public_key:[32]` |

Any other version or kind, a body that does not parse to exactly
`body_len` bytes, or bytes after the last signature reject the statement.

### What is signed and hashed

```text
signing_input  = "werewolfproxy/governance/v1/sign\0" || core
statement_hash = BLAKE3("werewolfproxy/governance/v1/hash\0" || core)
```

Both cover the core only, never the signatures. Two copies of the same
action with different sets of valid signatures therefore have the same hash
and are the same statement, so adding a signature can never create a fork.
The domain strings keep these bytes from ever being valid as a handshake,
RPC or Pelt signature input, and vice versa.

## Verification

A node verifies a statement against its head (or, for genesis, against
nothing) in this order and applies it only if every step passes.

1. **Shape.** Encoding as above; size limit; known version and kind.
2. **Chain.** For genesis: `sequence = 0`, `previous` all zero, and the node
   has no head yet. Otherwise: `network_id` equals the head's,
   `sequence = head.sequence + 1` and `previous = head.hash`.
3. **Signatures.** `count ≥ threshold` of the key set in force *before* this
   statement (genesis: its own body's keys and threshold); key indices
   strictly increasing and below `key_count`; every listed signature
   verifies with Ed25519 strict verification (`verify_strict`: no
   small-order or non-canonical keys or signatures) over `signing_input`.
   One bad signature rejects the statement, even if enough others are good:
   a statement is valid or not, never "valid after discarding parts".
4. **Body rules.**
   - Genesis: `threshold = 7`, `key_count = 10` (decision 1a's 7-of-10;
     other values are rejected in Stage28), the ten keys distinct and each a
     valid, non-small-order Ed25519 point.
   - Key rotation: 1–10 entries, indices strictly increasing and below 10,
     each new key valid, not small-order, and distinct from every key in
     the resulting set.
   - Network revocation: 1–64 keys, strictly increasing byte order (so a
     statement has one encoding), each a valid Ed25519 point.

### Statements that are not next

- **Already applied:** a statement whose sequence is at or below the head
  and whose hash equals the hash the node recorded for that sequence is
  reported as `already_applied` and changes nothing.
- **Fork:** a statement that passes steps 1 and 3 for a sequence the node
  has already applied, with a *different* hash, is evidence that the
  threshold signed two different histories. The node records it, sets
  `governance.forked`, and refuses every later statement until an operator
  intervenes (decision 5a). It never chooses between the branches.
- **Gap:** a sequence above `head.sequence + 1` is refused as `out_of_order`;
  the operator applies the missing ones first.

## Effects

- **Genesis:** creates the head (network ID, sequence 0, hash, keys,
  threshold 7, empty revoked list). The operator must supply the expected
  statement hash when applying it (decision S1), which pins the network the
  node joins.
- **Key rotation:** replaces the keys at the given indices for every later
  statement. Signatures on this statement were checked against the old set.
- **Network revocation:** adds the keys to the revoked list. Any Pack member
  with one of those public keys is revoked exactly as `pack revoke` does
  (authority denial first, its Fangs and every Fang using it as relay or
  provider closed), and from then on `pack add` of a revoked key is refused
  with `PACK_REVOKED_BY_NETWORK`. This is the only local operation Stage28
  restricts (decision 6a): local administration can still revoke more,
  never re-trust what the network revoked.

## Persistence

The head is a new protected document `governance` in the Stage15B manifest:
network ID, sequence, head hash, the ten keys, threshold, the revoked list,
the recorded hash of every applied sequence (32 bytes each), and the fork
flag. Persist first, then publish: a statement takes effect only after its
manifest generation commits; an indeterminate commit degrades storage as
for any protected mutation. Adding a document is a manifest schema change
(decision S2). The statements themselves are kept, outside the manifest, in
`governance/NNNNNNNN.wwgov` for audit.

## Operator surface (draft)

```text
werewolfctl governance apply FILE [--expect-hash HEX]   # genesis needs --expect-hash
werewolfctl governance status                            # head, keys, revoked, forked
```

Offline tool (decision 2a; no daemon, no network access):

```text
werewolfctl governance keygen OUT              # one holder's key file, 0600
werewolfctl governance draft genesis|rotate|revoke ... --out DRAFT
werewolfctl governance sign DRAFT KEYFILE --out PART
werewolfctl governance combine DRAFT PART... --out STATEMENT
werewolfctl governance inspect FILE            # decode and show the hash
```

## Decisions for this spec

Accepted 2026-10-23: S1 required `--expect-hash`, S2 manifest schema 2, S3
fence governance only, S4 BLAKE3 with domain strings.

- **S1. Pinning genesis (recommended: required `--expect-hash`).** The
  genesis hash is the network's root of trust, exchanged out of band like a
  Pelt key. Without pinning, anyone who can hand an operator a self-signed
  genesis decides which keys govern that node.
- **S2. Manifest schema (recommended: schema 2 with a `governance`
  document).** Binaries keep reading schema 1 manifests (no governance) and
  write schema 2 once genesis is applied; older binaries refuse schema 2,
  which is the safe direction.
- **S3. Fork response (recommended: fence governance only).** A fork sets
  `governance.forked` and stops statement processing; it does not enter
  Silver, because a fork says the governance keys are compromised, not that
  this node is.
- **S4. Hash function (recommended: BLAKE3 with domain strings).** The same
  construction as the Stage15B state commitment, already a dependency.

## Tests required before merge

- Test vectors: fixed seeds for ten governance keys and two members, the
  exact `core`, `signing_input`, `statement_hash` and statement bytes for a
  genesis, a rotation and a revocation.
- Negative tests for every rule: wrong version or kind, trailing bytes,
  short and long bodies, 6 signatures, duplicate or unordered indices, one
  bad signature among 8 good ones, non-strict signature, small-order key,
  repeated key in genesis, rotation to an existing key, revocation list out
  of order, wrong network ID, gap, replay, fork.
- Persistence: head survives restart; manifest rollback detected; a
  revoked key cannot be re-added; revocation closes routed and provider
  Fangs that use the member.
