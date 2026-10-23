# Stage28 governance and lifecycle — design survey

**Status: decisions accepted 2026-10-23 (1a, 2a, 3a, 4a, 5a, 6a, 7a; see
[Decisions](#decisions-accepted-2026-10-23)).** Nothing is implemented. The
statement format is specified for review in
[STAGE28_GOVERNANCE_STATEMENT_SPEC.md](STAGE28_GOVERNANCE_STATEMENT_SPEC.md)
before any code.

[ARCHITECTURE.md](ARCHITECTURE.md) fixes the goals and leaves the
mechanisms open: physical-key governance at 7 of 10 (§25–26, §61), no
permanent root after activation (§27–29, rule 13), an explicit lifecycle
with Silver distinct from Death (§35–36), and deliberate death that leaves
only a Tombstone (§32–34, §62, rules 15–16). §65 puts this after
multi-hop routing, which Stage26–27 completed.

## Where we are

Every node is sovereign: its operator (whoever owns the Den and the control
socket) adds and revokes Pack members and grants targets locally. There is
no notion of a network above the nodes: no network identifier, no shared
roster, no statement that more than one node must accept. Silver is the only
lifecycle state beyond READY, and it is per node and reversible.

What governance must add is a small, deliberate layer of **network-level
statements** that every node verifies and that no single person, key or
host can produce.

## Questions and options

### 1. The threshold mechanism

**1a. k-of-n multisignature (recommended).** A governance statement carries
independent Ed25519 signatures from at least 7 of the 10 registered
governance keys over the same canonical bytes. Verification reuses the
signature primitive the project already relies on; no new cryptography, no
interactive protocol, and the record shows exactly which keys signed. Cost:
7 × 64 bytes per statement, irrelevant for rare ceremonies.

**1b. Threshold signatures (FROST over Ed25519).** One aggregate signature,
signer set hidden. Needs distributed key generation, an interactive signing
round and a new cryptographic dependency with its own review; and a
compromised DKG is hard to detect. Not justified for a handful of
statements a year.

**1c. Secret sharing of one master key (Shamir).** Rejected: the key is
reconstructed whole at each ceremony, which recreates the single root
credential §27–28 exist to eliminate, if only for minutes, on one machine.

### 2. Custody of the physical keys

**2a. The daemon never holds a governance private key (recommended).**
Governance keys are Ed25519 public keys in the node's configuration; signing
happens offline in a separate tool, one holder at a time, and the signed
statement is carried to the nodes. Custody (hardware token, offline machine,
paper) is outside the daemon and can change without a protocol change.
**2b.** Additionally define a hardware-token signer (PIV or OpenPGP Ed25519)
now. Recommended later, as an implementation of 2a's tool, after the format
is fixed. §24 (no TPM dependency) holds either way.

### 3. Which actions need a ceremony first

Candidates, in rising order of consequence:

| Statement | Effect |
|---|---|
| **Genesis** | Creates the network: network ID, the 10 governance public keys, threshold 7, initial member roster |
| **Key rotation** | Replaces lost or compromised governance keys, still needing 7 valid current signatures |
| **Network revocation** | Removes a member network-wide; every node drops it from its Pack and closes its Fangs |
| **Roster admission** | Adds a member network-wide |
| **Activation** | Ends bootstrap; afterwards the gated actions in decision 6 need statements |
| **Death warrant** | Starts the terminal lifecycle (decision 7) |

**3a (recommended).** First stage: genesis, key rotation and network
revocation. Revocation is the action whose absence hurts most today (one
compromised member must be removed node by node) and whose failure mode is
safe (a node that never sees the statement keeps its old, more permissive
state, but cannot be tricked into trusting more). Admission, activation and
death follow in later stages. **3b.** All six at once: one large review
instead of three bounded ones.

### 4. How statements reach nodes

**4a. Operator-delivered first (recommended).**
`werewolfctl governance apply FILE`; the daemon verifies and applies it
locally. Simple, auditable, and works when the network is partitioned.
**4b.** Also gossip statements between Pack members over the Stage24 RPC
channel, so one delivery reaches everyone. Valuable for revocation speed;
recommended as a follow-up once the format is fixed, because a gossip path
is a new remotely reachable parser.

### 5. Ordering, replay and rollback

**5a (recommended).** Every statement carries the network ID, a sequence
number and the hash of the previous statement, forming a chain rooted at
genesis. A node applies only the next statement in its chain, records the
latest applied sequence and hash inside the Stage15B protected manifest
(so a Den rollback is detected like any protected-state rollback), and
refuses forks: two different statements with the same sequence are evidence
of key compromise and put the node into a fenced state for an operator to
inspect. **5b.** Timestamps and validity windows only: simpler, but replay
and reordering depend on clocks, which §31's partition tolerance cannot
assume.

### 6. What "root becomes unobtainium" means in practice

The honest boundary first: OS root on a host can always read and change
that node's Den (see [known limitations](../KNOWN_LIMITATIONS.md)). Governance
cannot make one host's root powerless over that host; it can make that root
powerless over the **network**: no single host, operator or key can admit a
member network-wide, rotate governance, or keep a revoked member trusted on
other nodes.

**6a (recommended for now).** Governance gates only network-level changes
(the statements of decision 3). Local administration stays as it is, and a
node's operator can still revoke more (never trust more than the network
roster allows, once admission exists). **6b. After activation, lock local
trust-widening mutations** (`pack add`, target grants to non-roster members)
behind statements. This is the full §27 goal; it changes daily operation
and recovery from mistakes, so it needs its own design after 6a has run.

### 7. Death and the Tombstone

Open problems that need their own design, recorded here so they are not
lost:

- **Who detects "unrecoverable"?** Governance keys are offline by design
  (decision 2a), so nodes cannot observe that fewer than 7 remain. Death is
  more likely a signed **death warrant** (7 of 10, decision 3) or an
  operator-armed dead-man condition than an automatic detection.
- **Anti-forensics is limited by storage.** Overwriting files on SSDs,
  journaling or copy-on-write filesystems and snapshots does not guarantee
  erasure. What is achievable is cryptographic erasure: keep Den secrets
  encrypted under a key that exists only in memory or one small file, and
  destroy that key. The claim must be stated that narrowly.
- **What the Tombstone holds:** network ID, terminal sequence, time, and
  the death warrant's signatures; nothing about members, routes or services.

**7a (recommended).** Defer Death to its own stage after activation, with
cryptographic erasure designed in from the start of that stage. **7b.**
Design it now alongside the first stage: possible, but it couples the most
destructive feature to the least proven format.

## Proposed stages

| Stage | Scope |
|---|---|
| 28 | Statement format (canonical encoding, network ID, chain, k-of-n signatures), genesis, key rotation, network revocation; daemon verifies and applies; offline signing tool with file keys; protected-manifest chain head; fork fencing; lab gate with ten generated keys |
| 29 | Roster admission, activation, and the 6b lockdown design; gossip delivery (4b) |
| 30 | Death warrant, cryptographic erasure of the Den, Tombstone |

## Security notes

- Statements only narrow trust in Stage28 (revocation) or reorganize
  governance (rotation); none widens what a node accepts.
- A node that misses a statement stays on its previous chain head and
  refuses later statements until it receives the gap, so it never applies a
  revocation out of order.
- Forks are treated as compromise, never resolved automatically.
- Seven compromised governance keys defeat the scheme by definition; the
  design makes that the only way.

## Decisions (accepted 2026-10-23)

1. **1a:** k-of-n Ed25519 multisignature, 7 of 10.
2. **2a:** the daemon never holds governance private keys; signing is offline.
3. **3a:** Stage28 covers genesis, key rotation and network revocation.
4. **4a:** statements are delivered by the operator first.
5. **5a:** hash chain from genesis, head in the protected manifest, forks
   fenced.
6. **6a:** governance gates network-level changes only for now.
7. **7a:** Death and the Tombstone are a later stage.

The member roster mentioned with genesis in the table above moves to
Stage29 with admission; Stage28's genesis carries only the governance keys
and threshold.
