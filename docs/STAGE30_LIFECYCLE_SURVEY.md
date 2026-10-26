# Stage30 network lifecycle, death and the Tombstone — design survey

**Status: decisions accepted 2026-10-26 (1a, 2a, 3a + 3a-ii, 4a, 5a, 6a,
7a, 8) with two owner additions; see
[Decisions](#decisions-accepted-2026-10-26).** Docs only. The format and
rules go into a spec for review before any code, as in Stage28.

[ARCHITECTURE.md](ARCHITECTURE.md) sets the goals:

- an explicit lifecycle that keeps Silver distinct from Death (§35–36);
- a distinction between temporarily unavailable and cryptographically
  unrecoverable (§31);
- deliberate death that destroys secrets and leaves only a Tombstone
  (§32–34, §62, rules 15–16).

The [Stage28 survey](STAGE28_GOVERNANCE_SURVEY.md) (§7, decision 7a)
deferred all of it to this stage. Stage29 completed activation and
governance sync, so the chain that death must end now exists.

This survey answers eight questions:

1. the lifecycle states and transitions;
2. objective criteria separating DEGRADED from UNRECOVERABLE;
3. whether death is ever automatic;
4. what is destroyed and what survives;
5. the Tombstone format;
6. how an offline node learns of death;
7. crash semantics;
8. honest testing.

## Where we are

- **The chain.** Statements are signed by at least 7 of 10 Ed25519
  governance keys. There are five kinds: 0 genesis, 1 rotation, 2 network
  revocation, 3 admission, 4 activation.
  - Every node verifies each statement and commits its head (network ID,
    sequence, hash, keys, revoked, roster, `activated_at`, `forked`) in the
    protected manifest before acting on it.
  - Statements reach nodes from a file (`governance apply`) or from Pack
    peers (`governance sync`).
- **The network ID** is 32 random bytes chosen when the genesis is drafted
  (`draft genesis` takes `--network-id` to override it). Nodes pin a genesis
  by its hash.
- **Governance keys are offline** (Stage28 decision 2a). No daemon ever
  holds one, so no node can observe how many keys still exist or work.
- **Node-local states today:**
  - Silver: per node, reversible, set by the operator;
  - unadmitted: derived from the head and the node's Pelt (Stage29);
  - a fork fence: durable, set when two valid statements conflict.
- **What a Den holds** (all under one directory):

  | Item | Files |
  |---|---|
  | Pelt (node identity, Ed25519 secret) | `pelt.json` |
  | Protected security state, slots A/B | `security_state_{a,b}_{pack,target-policy,silver,fangs,active-fangs,governance,manifest}.json`, `security_state_current.json`, `security_state_manifest_mode.json` |
  | Den state | `routes.json`, `services.json`, `published.json`, `advertise.json`, `contacts.json`, Fang profiles |
  | Governance archive | `governance_NNNNNNNN.wwgov` (every applied statement: governance keys, roster, revoked keys) |
  | Runtime | control socket, instance lock |

  The TLS identity is derived from the Pelt in memory
  (`tls_identity::from_pelt`); no TLS key is ever written. Session keys
  exist only in memory. Logs go to stderr, which means journald under the
  systemd unit, outside the Den.

## The central finding

> **A signature proves that a quorum exists. Nothing can prove that one
> does not.**

Seven valid signatures on a statement prove that at least seven usable
keys existed when it was signed. No software observation can prove that
fewer than seven exist: a missing signature may mean a lost key, or a key
holder on holiday. Two things follow, and they shape every answer below:

1. **UNRECOVERABLE can never be a chain fact.** Any statement that records
   it would need seven signatures, and those signatures would prove it
   false.
2. **A network that is really unrecoverable cannot sign its own death.** If
   death needs 7 of 10 and fewer than 7 keys remain, no death statement can
   ever exist. §32 ("if the network cannot recover legitimately, it must
   die deliberately") cannot be met by a network-wide statement in that
   case. It needs a separate, node-level answer (question 3).

§62's flow begins "detect unrecoverable condition → enter terminal
lifecycle". Nodes cannot do that detection. This survey proposes amending
§62 so that holders determine the condition and death is entered by
statement (decision 3).

## The owner's distinction

```text
DEGRADED       = governance capability reduced, network still valid
UNRECOVERABLE  = the governance quorum cannot be reconstructed under the rules
DEATH          = a valid terminal governance decision
TOMBSTONE      = post-death residual state
```

Fewer than seven keys being *available right now* is never the same as
"destroy everything".

## Questions and options

### 1. Lifecycle states and transitions

ARCHITECTURE §36 draws one state machine:

```text
BOOTSTRAP → CEREMONY → ACTIVE ⇄ SILVER
                         ├→ DEGRADED
                         └→ UNRECOVERABLE → DEATH → TOMBSTONE
```

It mixes three different kinds of thing:

- facts every node shares (the chain);
- node-local conditions (Silver, unadmitted);
- observations about the key holders.

A node can be in Silver while the network is degraded, and an unadmitted
node lives on an active network. A single machine cannot express those
combinations without inventing product states.

**1a. Three independent axes (recommended).**

**Chain lifecycle.** It is derived from the head, so it is identical on
every node that applied the same statements. Statements are its only
transitions.

| State | Meaning | Entered by |
|---|---|---|
| NONE (§36 BOOTSTRAP) | No genesis applied | Initial |
| CEREMONY | Genesis applied, not activated | Genesis (kind 0) |
| ACTIVE | Activated | Activation (kind 4) |
| DEAD | Terminal | Death statement (new kind 5), from CEREMONY or ACTIVE |

- NONE → CEREMONY → ACTIVE, with CEREMONY or ACTIVE → DEAD.
- Nothing leaves DEAD. No statement chains after it: a statement whose
  previous is a death statement is invalid on every node, so death cannot
  be undone or followed.
- A node with no genesis (NONE) cannot receive a network death; it can
  only retire locally (question 3).

**Node condition.** This is local to one node and depends on its own Den.

| Condition | Meaning | Reversible |
|---|---|---|
| NORMAL | Serving | — |
| SILVER | Operator emergency lock; state remains recoverable (§35) | Yes, `silver off` |
| UNADMITTED | Derived: activated chain, own Pelt not on the roster or revoked | Yes, by admission |
| FORK-FENCED | Two valid conflicting statements seen | No (Stage28) |
| DYING | DEAD committed, destruction not yet complete | No; only moves to TOMBSTONE |
| TOMBSTONE | Destruction complete; only the Tombstone remains | No, terminal |

- SILVER, UNADMITTED and FORK-FENCED combine with each other. The inbound
  lock is held if SILVER or UNADMITTED is set; FORK-FENCED only refuses
  further governance statements.
- DYING and TOMBSTONE override everything else.
- DYING is entered by applying a death statement. Under 3a it can also be
  entered by local retirement; see question 3.

**Governance capability.** This is an observation about the key holders.
**It never changes a node's behaviour.** The values are HEALTHY, DEGRADED
and UNRECOVERABLE, defined in question 2. It is assessed by holders and
recorded off-chain; nodes do not store it.

**1b.** Keep §36's single machine. It is simpler to draw, but it cannot
represent the combinations above, and it puts UNRECOVERABLE in the
transition path to DEATH, which is exactly the coupling the owner
distinction rules out.

### 2. DEGRADED versus UNRECOVERABLE: objective criteria

A key index is **usable** if its holder still has the private key, can
sign, and the key is not believed compromised. A compromised key counts as
unusable for a legitimate quorum: it can still sign, but not legitimately.

| Capability | Criterion |
|---|---|
| HEALTHY | All 10 indices usable |
| DEGRADED | At least 7 and fewer than 10 usable. Recoverable: rotation (kind 1) replaces lost or compromised keys while 7 can still sign |
| UNRECOVERABLE | Fewer than 7 usable. The rules allow no other recovery path, so this is permanent |
| COMPROMISED (noted, not a lifecycle state) | 7 or more keys in hostile hands: the attacker can sign anything, death included. The design cannot defend against this, by definition |

**UNRECOVERABLE is a legitimate-custody determination, not cryptographic
proof that private keys no longer exist** (owner clarification). It means
that fewer than seven indices remain in the custody of their legitimate
holders, so no legitimate quorum can form under the rules. A key reported
lost may still exist: in a forgotten backup, or in an attacker's hands, and
it can still sign. Three consequences:

- UNRECOVERABLE never destroys anything by itself (decision 3);
- every loss report is also treated as a possible compromise: a lost key
  that reappears in hostile hands counts toward COMPROMISED, not toward
  recovery;
- nothing in the software, and no test, claims that a lost key is gone.

The criteria are objective, but the evidence is held by people. The
question is how to measure the number of usable keys without trusting one
person's word.

**2a. Key check plus loss reports, held off-chain (recommended).**

- **A key check is a lower-bound measurement.** A new offline command signs
  a fresh challenge with one governance key:
  - the challenge is random and chosen by whoever runs the check;
  - the signature covers the challenge and the network ID;
  - it uses its own signature domain, so a check signature can never serve
    as a statement signature.

  N distinct valid check signatures prove at least N usable keys **now**.
  A check with 7 or more proves the network is not unrecoverable.
- **A loss report is an affirmative statement by a holder** ("index i is
  destroyed, lost or compromised"), recorded in the holders' register.
- **The decision rule:**
  - **at least 4 indices reported lost** ⇒ at most 6 usable ⇒
    UNRECOVERABLE;
  - **a check with fewer than 7 signatures, but fewer than 4 reported
    losses** ⇒ *unverified*, never UNRECOVERABLE: some holders may simply be
    absent, so repeat the check;
  - **a check with 7 to 9 signatures, or 1 to 3 reported losses** ⇒
    DEGRADED, and the remedy is rotation now.
- **Last-quorum rule (procedural).** When exactly 7 usable keys remain, the
  holders must, before anything else, either rotate in replacements or
  sign a death statement while they still can. It is the last moment at
  which either is possible.

Nodes never see any of this; it is holder procedure backed by a
verifiable measurement.

**2b. Record capability on the chain.** A 7-of-10 "key status" kind would
mark indices lost, and every node would show DEGRADED. Rejected for Stage30:

- it can record DEGRADED but, per the central finding, never UNRECOVERABLE;
- rotation already fixes DEGRADED better than recording it does.

**2c. Liveness leases.** The network would need a 7-of-10 "renewal" every
period T, and nodes would treat a missed renewal as UNRECOVERABLE.
Rejected:

- it depends on host clocks, which an attacker on the host controls;
- it turns holder absence into a network state.

### 3. Is death always an explicit 7-of-10 act?

**3a. Explicit 7-of-10 death statement only (recommended, the owner's
bias).**

- Death is a new kind 5 with an empty body, signed like every other
  statement and chained to the current head.
- UNRECOVERABLE is an observation and **never** triggers destruction.
- Six available keys today may be seven next month, when a holder returns.

**3b. Automatic on detected conditions** (the literal §62). Rejected:

- nodes cannot detect the condition (central finding);
- every automatic proxy (timeouts, missed renewals, partitions) lets an
  attacker who controls clocks or the network destroy a healthy network.

**3c. A pre-signed death warrant held in escrow**, signed while quorum
exists and valid at any head. Rejected as a default:

- it is a standing kill switch, and its theft is the network's death;
- it must break the chain rule ("valid at any head") to stay usable.

**The gap 3a leaves, and how to close it.** Under 3a an UNRECOVERABLE
network cannot die by statement. Two measures close it:

- **The last-quorum rule (2a).** Holders act while seven keys still exist.
- **3a-ii. Local retirement (recommended).** A node-local command
  (proposed `den retire`, requiring explicit confirmation) runs the same
  destruction and crash semantics as death on this one node:
  - it needs no quorum and no network state;
  - it writes a *retirement* Tombstone, which records no death statement
    and is not a network Tombstone;
  - it only narrows trust, like `pack revoke` and Silver, so it fits
    decision 4a of Stage29: local actions that narrow trust stay local.

  An unrecoverable network then winds down node by node, each operator
  acting deliberately (§32). It also serves NONE nodes and any node an
  operator wants to decommission.

**Proposed ARCHITECTURE amendments** (decision 3):

- §36: replace the single machine with the three axes of 1a.
- §62: "detect unrecoverable condition" becomes "holders determine the
  condition (2a); death is entered by a 7-of-10 statement while quorum
  exists, otherwise each node retires locally".

### 4. What is destroyed and what survives

On death (or retirement), the daemon destroys everything Werewolf wrote:

| Item | Secret | Reconstruction value | Action |
|---|---|---|---|
| Pelt (`pelt.json`) | Yes, the node's only long-lived secret | Identity | **Destroyed first** |
| TLS identity, session keys | Yes | — | Memory only; the process ends after destruction |
| Protected security state (both slots, current pointer, manifest mode) | No secrets | Pack (peer keys and addresses), grants, Fang list, governance head | Destroyed |
| Den state (`routes`, `services`, `published`, `advertise`, `contacts`, Fang profiles) | No | Topology, services, addresses | Destroyed |
| Governance archive (`governance_*.wwgov`) | No | Governance keys, roster, revoked keys | Destroyed |
| Control socket, instance lock | No | — | Removed |
| The Den directory | — | — | Emptied except the Tombstone |

**Survives:** the Tombstone (question 5), the installed binaries and the
service unit. Neither the binaries nor the unit are secret, and neither is
specific to the network.

**Outside Werewolf's control:** these are never claimed. They are listed
as operator procedure in the death runbook.

- journald and any other log sinks (rotation or vacuum is the operator's
  job);
- swap and hibernation images;
- filesystem journals, copy-on-write snapshots, SSD wear-levelling and
  remapped blocks;
- backups, VM snapshots and copies made by anyone;
- development tool caches (`~/.cache/werewolf`);
- **the governance private keys themselves**, which holders destroy as the
  last step of the death ceremony; no software can verify it.

**How files are destroyed:**

**4a. Overwrite, sync, unlink, then sync the directory (recommended for
Stage30)**, with a bounded claim:

> "Werewolf removes every file it created in the Den and overwrites it
> first. It does not and cannot guarantee forensic erasure of storage
> media."

On ext4 with journaling, on copy-on-write filesystems and on SSDs, the
overwrite may not reach the original blocks. The claim says so.

**4b. Cryptographic erasure** (raised in the Stage28 survey). Den state
would be encrypted at rest under one data key, and only that key destroyed.
This has two parts:

- **The benefit:** remnants of Pack and topology files become ciphertext.
- **The limit:** the data key is itself a small file with exactly the
  Pelt's storage problem, unless it lives only in memory, which needs an
  unlock at every start: a passphrase or a TPM, and §24 rules out a TPM
  dependency.

It also changes the persistence format of the protected manifest.
**Recommended as a separate later stage, not Stage30**, so that death does
not wait on an at-rest encryption design. The bounded claim of 4a stays
true either way.

### 5. The Tombstone

The Tombstone is one small file, `tombstone.json`, in the otherwise empty
Den, written atomically.

**5a. Minimal and verifiable with the holders' keys (recommended).**

| Field | Content | Why |
|---|---|---|
| `format` | `"werewolf-tombstone"`, `version: 1` | Self-describing |
| `kind` | `"death"` or `"retirement"` | Network death vs local retirement (3a-ii) |
| `network_id` | 32 bytes hex (death only) | That a network existed (§34) |
| `terminal_sequence`, `terminal_hash` | The death statement's sequence and hash (death only) | The terminal head |
| `death_statement` | The kind-5 statement, base64 (death only) | Attestation of the terminal event: its signatures cover the network ID and the previous head |
| `destruction` | `"complete"` or `"incomplete"` | Honest record (question 7) |
| `software` | The `werewolfd` version that executed it | Protocol metadata |

**Excluded, and why each could reconstruct something:**

- the node's own Pelt key or fingerprint, which links the host to the
  network;
- governance public keys, which make the Tombstone self-verifying but
  publish the key set and link holders across networks;
- the roster, revoked keys, the Pack, peers and addresses, routes,
  services, grants, Fang profiles and contacts;
- earlier statements;
- free text: kind 5 has an empty body, so no reason or comment field
  exists anywhere.

**Timestamps.** 5a records none, because host clocks are untrusted and a
time is evidence. **5b** would add a local, explicitly non-authoritative
time. It helps operators, but is a small forensic leak. The owner decides.

**Verification.** Anyone holding the governance public keys (the holders)
can verify `death_statement`. The Tombstone alone proves only internal
consistency: `terminal_hash` is the hash of `death_statement`.

**Linkage that remains (stated, accepted):**

- `network_id` and `terminal_hash` let someone who already holds other
  artefacts of this network, such as a seized statement archive, link them
  to this Tombstone. That is inherent in "a network existed".
- The death statement's signer indices show which 7 or more indices
  signed.

**"Proving" no reconstruction material.** The proof is not a claim about
the Tombstone. It is three mechanical checks:

1. an exact schema: unknown fields are rejected on write and on read, and
   the size is bounded (a few KiB, set by the maximum statement size);
2. a field-by-field argument, the table above, in the spec;
3. a test that scans the Tombstone and the whole Den for every key, address,
   port, name and service from a pre-death snapshot, in raw, hex and base64
   forms (question 8).

**No resurrection:**

- DEAD is terminal in the chain rules (question 1).
- A Den holding a Tombstone refuses `den init` and `pelt init`. Root can
  delete the directory, and the claim does not pretend otherwise.
- A new genesis is a new chain with a new genesis hash, even if someone
  forces the old network ID with `--network-id`. Nodes pin the genesis
  hash, so it can never continue the dead chain.

**5c. Self-verifying Tombstone** (include the key set). Rejected for the
linkage reasons above.

### 6. How an offline node learns of death

**6a. The authenticated chain, as for every statement (recommended, the
owner's bias).**

- **The primary path:** the holders distribute the death statement file to
  every operator, exactly as they did the genesis.
  - It is self-authenticating; anyone may carry it, over any channel.
  - The operator runs `governance apply`.
- **The secondary path:** ~~`governance sync` from a peer that has not died
  yet~~. *Corrected in Stage30 step 2:* there is no such window. A
  responder serves only applied statements, and applying a death locks
  and destroys the node first, so sync can never deliver a death. The file
  is the only channel (see the spec's step 2 notes).
- **What the node does:** having learned of death, it performs its own
  terminal transition (question 7).
- **A node that never learns:**
  - It keeps its secrets, but its peers are dead or retired, so it can do
    nothing with them.
  - Its operator retires it locally (3a-ii). The death runbook lists every
    node and requires a confirmation from each.
- **Later work:** ~~automatic sync (next on the owner's order) will
  propagate death like any other statement.~~ *Corrected in Stage30 step
  2:* death **is** an ordinary chain statement, but no node can serve it,
  so automatic sync cannot deliver it either (see the
  [Stage31 survey](STAGE31_AUTO_SYNC_SURVEY.md#7-death)).

**6b. Two-phase death.** A DYING node keeps its Pelt and answers only the
`governance` RPC for a bounded window, measured on the monotonic clock, so
that offline peers can still fetch the death statement. This widens
propagation but extends the period in which secrets exist after the
decision. It is not recommended; the file is the reliable channel.

**6c. An unauthenticated beacon.** A dead node's listener would answer
anyone with the death statement. Rejected:

- the host stays fingerprintable as a former Werewolf node;
- a listener survives death, contrary to §33.

### 7. Crash and interruption semantics

**7a. Persist the decision first, then destroy in a fixed, idempotent
order (recommended).**

1. **Verify and archive** the death statement (Stage29 G3), then **commit
   the DEAD head** in the protected manifest. *Persist before publish.*
   Until this commit the node is not dying, and a crash leaves it exactly
   as before.
2. **Lock everything at once.** This uses Silver's lock, plus the fence on
   outbound opens:
   - inbound sessions and RPCs are refused;
   - Fangs and listeners are closed;
   - every control command except `status` is refused.
3. **Write `tombstone.json`** atomically (temporary file and rename) with
   `destruction: "incomplete"`. It contains only public data from the
   statement.
4. **Destroy, secrets first:**
   - the Pelt;
   - then the protected security state;
   - then the Den state;
   - then the governance archive.

   Every step succeeds if its target is already gone.
5. **Rewrite the Tombstone** with `destruction: "complete"` (atomic
   replace), then exit with a distinct exit code.

**Startup:**

- **A Tombstone exists:** rerun step 4 (idempotent), finish step 5, and
  never become READY. The daemon reports the Tombstone and exits.
- **A DEAD head is committed but there is no Tombstone** (a crash between
  steps 1 and 3): continue from step 2.
- **Any other state:** normal startup.

**Failure:**

- A destruction step that fails (an I/O error, a read-only filesystem)
  leaves the node DYING:
  - locked, never READY;
  - the Tombstone says `"incomplete"`;
  - it retries at every start.
- Destroying the Pelt first means that even a stuck destruction leaves a
  node that can no longer authenticate anywhere.

Death goes through mutation admission like every governance mutation, so
it is linearized with other mutations. After the commit, every mutation is
refused.

**Local retirement (3a-ii; owner addition to 7a).** Retirement has no DEAD
chain-head commit to serve as its durable point of no return, so the
retirement Tombstone takes that role:

1. **Lock** in memory (as step 2 above). A crash here leaves the node
   exactly as before, since nothing durable has changed.
2. **Durably write the incomplete retirement Tombstone**
   (`kind: "retirement"`, `destruction: "incomplete"`). It is written to a
   temporary file, the file is synced, renamed into place, and the Den
   directory is synced. **Only after that sync succeeds** does any
   irreversible step begin. This is the point of no return. If the write or
   the sync fails, retirement fails with no effect and the node is
   unlocked.
3. **Destroy**, secrets first, exactly as step 4 above.
4. **Mark the Tombstone complete** (atomic replace), and exit.

At startup, a retirement Tombstone means exactly what a death Tombstone
means: resume destruction and never become READY.

The same durability applies to the death Tombstone in step 3 (file and
directory synced). For death, though, the DEAD head commit in step 1 is
already the point of no return.

**7b. Destroy first, then record.** Rejected: a crash would leave a
half-destroyed Den with no marker of intent, which startup could mistake
for a corrupt node or try to repair.

### 8. Testing destruction honestly

**What tests can establish, and what will be asserted as PASS or FAIL:**

- **The Den inventory.** After death, the Den contains exactly
  `tombstone.json`. This is an exact allowlist walk, not "the known files
  are gone".
- **Tombstone content:**
  - an exact schema and the size bound;
  - a scan of the Tombstone, and of every file left in the Den, for each
    secret, public key, address, port, peer name and service name from a
    pre-death snapshot, in raw, hex and base64 forms.
- **Behaviour after death:**
  - the daemon never becomes READY and refuses commands;
  - former peers fail to authenticate it (Fang and RPC from a peer both
    fail);
  - a second death or any later statement is refused.
- **Crash convergence:**
  - an abrupt kill at each step boundary (the D03 method);
  - storage faults: a read-only Den and a full disk (the D09 method).

  Every run converges to a complete Tombstone, or to a locked node marked
  `incomplete`, **never** to READY.
- **Propagation:**
  - death applied from a file;
  - death through sync;
  - a node offline during death, which learns later from the file;
  - local retirement.

**What tests will measure and report, but never claim as a pass:**

- **Media residue.** Run a node on a loop-mounted ext4 image, kill the
  network, unmount, and scan the raw image for the Pelt secret and the Pack
  keys. The finding (for example "the Pelt secret was found in 2 journal
  blocks") is recorded as a measurement of the filesystem, which 4a never
  claimed to defeat. A clean scan is reported as "not found on this run",
  never as "erased".

**What cannot be tested, and is never claimed:**

- SSD wear-levelling and remapped blocks;
- swap and hibernation;
- journald contents;
- backups and snapshots;
- memory remanence after the process exits;
- copies made by anyone;
- that holders destroyed their governance keys.

**The claim proposed for the documentation:**

> On death, Werewolf commits the decision, locks the node, destroys the
> node's identity key first and then every file it created in the Den, and
> leaves only a minimal Tombstone. It makes no claim about storage media,
> logs, swap, backups or copies outside the Den.

## Proposed steps

| Step | Scope |
|---|---|
| 0 | This survey; owner decisions 1–8 |
| 1 | Spec for review: kind 5 and terminal chain rules, Tombstone format and schema, destruction order and crash semantics, key-check signature format (if 2a is accepted), `den retire`; ARCHITECTURE §36/§62 amendments |
| 2 | Core: kind 5, the DEAD head, Tombstone encode and verify, offline key check |
| 3 | Daemon: DYING and TOMBSTONE, destruction, startup, `den retire`; lab gate with crash and storage faults, and the residue measurement |
| 4 | Real-host check on disposable daemons only |

Then, in the owner's order:

1. automatic governance sync, ~~which will propagate death as an ordinary
   statement~~ (*corrected in Stage30 step 2:* sync can never deliver a
   death; see the [Stage31 survey](STAGE31_AUTO_SYNC_SURVEY.md#7-death));
2. robustness and chaos certification: netem loss, jitter and reordering,
   and clock shifts;
3. the CLI architecture-complete candidate.

## Security notes

- Death only ever narrows: it removes authority and data, and grants
  nothing.
- **Seven compromised keys can kill the network.** That is the same bound
  as every other statement, and it is accepted: those keys could equally
  revoke every member.
- No timer, clock or network observation ever triggers destruction.
- Destruction cannot be used to roll back. DEAD is committed before
  anything is destroyed, and nothing chains after it.
- Local retirement can be done by anyone with the node's control socket,
  exactly like `pelt init` on an existing node today. It affects only that
  node.

## Decisions (accepted 2026-10-26)

The owner accepted every recommendation below, with two additions:

- **7a, retirement.** Local retirement must durably write its incomplete
  retirement Tombstone before the irreversible process begins, because,
  unlike network death, it has no DEAD chain-head commit to serve as its
  durable point of no return. See question 7.
- **2a, clarification.** UNRECOVERABLE is a legitimate-custody
  determination, not cryptographic proof that private keys no longer
  exist. See question 2.

5a was chosen over 5b: the Tombstone records no timestamp.

1. **States:** 1a, three axes (chain lifecycle, node condition, governance
   capability), with DEAD terminal and nothing chaining after it.
2. **Capability criteria:** 2a, a key check (lower bound) plus loss reports
   held off-chain; UNRECOVERABLE if and only if at least 4 indices are
   reported lost, a legitimate-custody determination, not proof that the
   keys no longer exist; the last-quorum rule.
3. **Death trigger:** 3a, an explicit 7-of-10 kind-5 statement only, plus
   3a-ii local retirement (`den retire`); amend ARCHITECTURE §36 and §62.
4. **Destruction:** 4a, overwrite and unlink with the bounded claim;
   cryptographic erasure (4b) is a later, separate stage.
5. **Tombstone:** 5a, minimal, with no timestamp; the exact schema and
   the scanning test as the proof.
6. **Learning of death:** 6a, the file first and sync second, with
   retirement for nodes that never learn.
7. **Crash semantics:** 7a, commit DEAD, lock, write the Tombstone as
   incomplete, destroy secrets first, then mark it complete; never READY
   again. Local retirement durably writes (file and directory synced) its
   incomplete retirement Tombstone before any irreversible step, as its
   point of no return.
8. **Testing:** assert the Den allowlist, Tombstone scanning, behaviour,
   crash convergence and propagation; *measure and report* media residue;
   claim nothing about storage, logs, swap or copies.
