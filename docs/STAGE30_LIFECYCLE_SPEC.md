# Stage30 death, retirement and the Tombstone — format and behaviour spec

**Status: specification approved 2026-10-26 (L1–L6 as recommended, the
Tombstone-before-destruction rule, and invariants I1–I3; see
[Decisions](#decisions-for-this-spec)).** No code exists yet. This implements the
accepted decisions of the [Stage30 survey](STAGE30_LIFECYCLE_SURVEY.md)
(1a, 2a, 3a + 3a-ii, 4a, 5a, 6a, 7a, 8 and the two owner additions). It
extends the [Stage28 statement spec](STAGE28_GOVERNANCE_STATEMENT_SPEC.md)
and the [Stage29 addendum](STAGE29_GOVERNANCE_ADDENDUM_SPEC.md); anything
not stated here is unchanged.

## Terms

- **Death statement:** a statement of kind 5. It is the network's terminal
  governance decision.
- **DEAD:** the chain-lifecycle state of a head whose last statement is a
  death statement.
- **DYING:** the node condition from the point of no return until
  destruction is complete.
- **Retirement:** local destruction of one node by its operator (3a-ii). It
  has no statement and no chain effect.
- **Tombstone:** the one file left in a destroyed Den, `tombstone.json`.
- **Point of no return:** the durable write after which destruction always
  completes, even across crashes:
  - **for death**, the commit of the DEAD head;
  - **for retirement**, the durable write of the incomplete retirement
    Tombstone (owner addition to 7a).

## 1. The death statement

### Encoding

| kind | Statement | body |
|---|---|---|
| 5 | Death | empty (`body_len = 0`) |

Encoding, signing input, hash, signature rules and size limits are exactly
those of Stage28. The body is empty, so a death statement carries no free
text, reason or comment.

### Chain rules

- **Death may follow any head**, CEREMONY or ACTIVE, at sequence
  `head.sequence + 1` with `previous = head.hash`, like every statement.
  There is no other constraint on what precedes it.
- **Nothing follows death.** A statement whose verification reaches a head
  that is DEAD is rejected with the new error `Terminal`, whatever its kind
  and signatures. This includes a second death statement. Re-presenting
  the same death statement is still `already_applied`.
- **Forks are unchanged** for a node that has not applied death: another
  valid statement at the same sequence as a death statement is a fork,
  exactly as in Stage28.
- **On a forked node** (L1): the only statement a forked node accepts is a
  death statement that chains to its own head. Death only narrows, and a
  forked node is otherwise stuck with live secrets forever. Any other
  statement is still `GOVERNANCE_FORKED`.
- **Death on a fork never resolves that fork (invariant I1).** Applying it:
  - keeps `forked` set in the committed DEAD head;
  - chooses no branch, and never clears, rewrites or reinterprets the fork
    record;
  - makes no other statement acceptable.

  The death statement is accepted only because it chains to the node's own
  head. It says nothing about which history was legitimate.

The head gains `dead_at: sequence`. It is omitted while absent, so Stage29
head documents keep their exact bytes. The document check adds:
`dead_at`, if present, equals the head sequence.

A Stage29 node rejects kind 5 (`UnknownKind`), so nodes are upgraded before
a death statement is delivered.

### Offline tool

```text
werewolfctl governance draft death --after PREVIOUS --out FILE
```

`inspect` shows `"kind": "death"`. Signing and combining are unchanged.

## 2. The key check (decision 2a)

A key check measures a lower bound on the number of usable governance keys
**now**. It never touches a daemon, and nothing in the network acts on it.

### Bytes

```text
key_check_input = "werewolfproxy/governance/v1/key-check\0"
                  || network_id:[32] || challenge:[32]
```

- The domain string differs from the statement signing domain
  (`…/v1/sign\0`) and from every handshake, RPC and Pelt domain, so a check
  signature can never be valid as a statement signature, or the reverse.
- The challenge is 32 bytes from the OS random generator, chosen by
  whoever runs the check, so old answers cannot be replayed.
- Signatures are Ed25519 and verified with `verify_strict`, as for
  statements.

### Files

A challenge:

```json
{"format":"werewolf-key-check","version":1,
 "network_id":"<64 hex>","challenge":"<64 hex>"}
```

An answer:

```json
{"format":"werewolf-key-check-answer","version":1,
 "network_id":"<64 hex>","challenge":"<64 hex>",
 "index":N,"signature":"<128 hex>"}
```

Parsing is strict: unknown fields, wrong lengths and non-lowercase hex are
rejected.

### Commands (offline)

```text
werewolfctl governance check new --chain FILE... --out CHALLENGE
werewolfctl governance check sign CHALLENGE KEYFILE --index N --out ANSWER
werewolfctl governance check verify CHALLENGE ANSWER... --chain FILE...
```

- **`new`** verifies the chain (genesis first, as `inspect --chain`) and
  takes the network ID from it.
- **`sign`** applies the same key-file checks as `governance sign`.
- **`verify`**:
  - checks every answer against the key in force at the chain's head for
    its index (rotations included);
  - rejects answers for another challenge or network, and duplicate
    indices;
  - reports the valid indices and their count.

  Its verdict is only:
  - `at_least_7_usable` when the count is 7 or more;
  - otherwise `unverified`.

  It never prints UNRECOVERABLE: that is a legitimate-custody
  determination from loss reports (survey, question 2), which is holder
  procedure, not software.

## 3. Node lifecycle

### States (decision 1a)

`governance status` and `status` gain:

```json
"lifecycle": {"chain": "none|ceremony|active|dead",
              "node": "normal|dying|tombstone"}
```

Silver, `admitted` and `forked` keep their existing fields. Governance
capability never appears; nodes do not know it.

### Applying death

Death arrives by `governance apply FILE` or through `governance sync`, and
goes through the ordinary statement path:

- mutation admission;
- verification against the live head;
- the archive written first (G3);
- the DEAD head committed in the protected manifest.

It is applied under Silver, on an unadmitted node, and (L1) on a forked
node.

Under L2 (recommended) there is no node-side confirmation flag. A death
statement is a 7-of-10 network decision, exactly like activation, and a
node operator has no veto over it.

**The point of no return is the DEAD head commit.**

- `DurablyCommitted`: destruction begins (section 4).
- `NotCommitted`: nothing happened; the statement fails as any failed
  apply does.
- `IndeterminateAfterRename`: as for every protected mutation, storage
  degrades and the node stays locked. At the next start, whatever head the
  manifest holds decides: DEAD means destruction, otherwise the node is
  unchanged.

The control reply is sent after destruction ends (section 4, step 5):

```json
{"status":"applied","sequence":N,"hash":"…","lifecycle":"dead",
 "destruction":"complete|incomplete"}
```

`governance sync` reports a death it applied in `applied` and stops, since
nothing chains after it.

### Retiring a node (3a-ii)

```text
werewolfctl den retire --confirm FINGERPRINT
```

- `FINGERPRINT` must equal the node's own Pelt fingerprint, typed in full.
  This is a deliberate-intent check, not a security boundary: whoever holds
  the control socket can read the fingerprint.
- A Den without a Pelt holds no identity; its operator simply removes it.
- Retirement works with or without governance, under Silver, unadmitted or
  forked.
- It goes through mutation admission. Once it starts, every other mutation
  is refused.

**The point of no return is the durable incomplete retirement Tombstone**
(owner addition):

1. **Lock in memory** (the Silver lock plus refusal of every mutation).
   Nothing durable has changed, so a crash here leaves the node exactly as
   before.
2. **Write `tombstone.json`** with `kind: "retirement"` and
   `destruction: "incomplete"`, through the Den's atomic replace (file
   sync, rename, directory sync).
   - `DurablyCommitted`: the point of no return. Continue with section 4,
     step 3.
   - `NotCommitted`: retirement fails with no effect
     (`RETIRE_SAVE_FAILED`), and the node unlocks.
   - `IndeterminateAfterRename`: no destruction begins. The node stays
     locked and storage degrades. At the next start, a Tombstone present
     means retirement resumes; an absent one means the node is unchanged.
     The operator asked for retirement either way, and nothing irreversible
     happened without a durable record.
3. **Continue with section 4, steps 3–5.**

The reply comes after destruction ends:
`{"status":"retired","destruction":"complete|incomplete"}`.

## 4. Destruction (decisions 4a and 7a)

The same procedure serves death and retirement. Every step is idempotent:
a target that is already gone counts as done.

1. **Lock everything at once:**
   - set the inbound lock (as Silver);
   - close every Fang and every listener;
   - refuse every control command except `status` and `governance status`;
   - mark the node DYING in memory.
2. **Write the incomplete Tombstone** (`destruction: "incomplete"`) by
   atomic replace (file and directory synced).
   - **No destruction step begins until this write is `DurablyCommitted`.**
     This applies to death too, even though DEAD is already committed: it
     guarantees that a Den whose security state is gone always carries a
     Tombstone, so it can never be mistaken for a fresh Den.
   - For death, a failed or indeterminate write leaves the node DYING and
     locked. The write is retried at the next start, because the committed
     DEAD head still drives it.
3. **Destroy in this order:**
   1. `pelt.json`, so that whatever follows, the node can no longer
      authenticate anywhere;
   2. every protected security state file (both slots, the current pointer,
      the manifest mode);
   3. every other entry in the Den, the governance archive and Fang
      profiles included;
   4. empty subdirectories, deepest first.

   **The target is everything in the Den directory except
   `tombstone.json` itself**, including a leftover temporary file from an
   interrupted Tombstone write. The daemon is the only writer, and no write
   is in progress during this step. It is not a list of known
   names, so files from older or future versions are covered. The Den is
   Werewolf's directory; the operator keeps nothing else in it.
4. **Destroying one entry** uses the Den's directory handle
   (`local_fs::PrivateDirectory`; every operation relative to the Den,
   never following a symlink):
   1. A `statat` with `SYMLINK_NOFOLLOW` acts as a pre-filter that **may
      only exclude**. A symlink, socket, FIFO, device or directory is never
      opened; it is unlinked (or, for a directory, removed when empty in
      step 3.4).
   2. A candidate regular file is opened relative to the Den with
      `WRONLY | NOFOLLOW | NONBLOCK | CLOEXEC`. A failed open makes it
      ineligible: it is unlinked only.
   3. **Overwrite eligibility is decided from `fstat` on that opened file
      descriptor, never from the pre-open `statat` (invariant I2).** Only a
      descriptor that is a regular file with `st_nlink == 1` and
      `st_uid` equal to the daemon's effective uid is overwritten. The
      overwrite is one pass of zeros over the full `st_size`, through that
      same descriptor, then synced. Any other result closes the descriptor
      unwritten.
   4. The entry is then unlinked by name.

   A file swapped between the pre-filter and the open is therefore judged
   by what was actually opened. Destruction never follows a link out of
   the Den or overwrites data that is reachable elsewhere (L3). After the
   last unlink, the Den directory is synced.
5. **Verify the Den against the exact allowlist**: the directory, read
   again from the start, contains `tombstone.json` and nothing else.
   - Only if it does is the Tombstone rewritten with
     `destruction: "complete"` (atomic replace).
   - Otherwise the Tombstone stays `incomplete`.

   Then remove the control socket and exit with exit code **67
   (`TOMBSTONE`)**.

**A Tombstone may only be, or remain, `complete` if the Den exact-allowlist
verification succeeds (invariant I3).** This holds at step 5 and at every
startup (section 6). A `complete` Tombstone found next to any other entry is
first durably rewritten as `incomplete`, and destruction then runs again.

**In memory:** the Pelt secret is already held in zeroizing containers
(`werewolf_core::pelt`). The process exits after step 5, which ends every
session key and the TLS identity derived from the Pelt.

**Failure:** any step that fails (an I/O error, a read-only filesystem, a
permission error) leaves `destruction: "incomplete"`. The node stays DYING
and locked, and the reply reports `incomplete`. The process exits with 67;
the next start retries. Destruction never skips past a failed Pelt removal
to report `complete`.

**Under systemd:** a service restarted on failure would loop on a
Tombstone. The packaged unit `packaging/systemd/werewolfd.service` adds 67
to its existing `RestartPreventExitStatus=64 65 66 69`, and a
Tombstone start exits at once (section 6), so even a loop does no harm.

## 5. The Tombstone (decision 5a)

### Schema

Exactly one JSON object with these fields in this order. No other field is
allowed on write or on read. The file is at most 8 KiB and has mode 0600.

| Field | Death | Retirement | Content |
|---|---|---|---|
| `format` | ✓ | ✓ | `"werewolf-tombstone"` |
| `version` | ✓ | ✓ | `1` |
| `kind` | ✓ | ✓ | `"death"` or `"retirement"` |
| `network_id` | ✓ | — | 64 lowercase hex |
| `terminal_sequence` | ✓ | — | integer |
| `terminal_hash` | ✓ | — | 64 lowercase hex |
| `death_statement` | ✓ | — | standard padded base64 of the full kind-5 statement |
| `destruction` | ✓ | ✓ | `"incomplete"` or `"complete"` |
| `software` | ✓ | ✓ | `"werewolfd " + CARGO_PKG_VERSION` (at most 64 printable ASCII) |

### Consistency (checked on write and on read)

- `death_statement` decodes, has kind 5, and its `network_id` and
  `sequence` equal the fields;
- its statement hash equals `terminal_hash`.

The signatures are **not** verifiable from the Tombstone alone: the
governance public keys are deliberately excluded (survey 5c). The holders
verify with `governance inspect` and their chain.

### Field-by-field: why nothing here reconstructs the network

| Field | What it reveals | Why that is not reconstruction material |
|---|---|---|
| `format`, `version`, `kind`, `destruction` | Constants and an enum | Carry no network data |
| `software` | The daemon version | Generic; not specific to this network or node |
| `network_id` | 32 random bytes chosen at genesis | Identifies the network that existed (§34) and nothing about its members, keys or topology |
| `terminal_sequence` | How many statements the chain had | A count only |
| `terminal_hash` | A BLAKE3 commitment to the death core | Reveals nothing about earlier statements beyond the `previous` link below |
| `death_statement` | `network_id`, `sequence`, `previous` (the hash of the last live statement), kind 5, an empty body, and 7–10 `(index, signature)` pairs | `previous` is a one-way hash. Signatures over a fixed message reveal neither the private keys nor, without the key set, the public keys. Indices show which holders signed |

**Not present:** the node's Pelt key or fingerprint, governance public
keys, the roster, revoked keys, Pack members, addresses, ports, routes,
services, grants, Fang profiles, contacts, earlier statements, timestamps
and free text.

**Linkage, accepted in the survey:** anyone already holding other
artefacts of this network, such as a statement archive, can match them to
`network_id` and `previous`.

## 6. Startup

Before loading any security state, the daemon checks the Den, in this
order:

1. **`tombstone.json` exists.**
   - Parse it strictly. A malformed Tombstone is never treated as absent:
     startup fails with `SECURITY_STATE`, exit 65, and the operator
     inspects.
   - If `incomplete`, run section 4, steps 3–5 (destruction resumes).
   - If `complete`, run the exact-allowlist verification (I3). If any other
     entry exists, or the directory cannot be read, durably rewrite the
     Tombstone as `incomplete` before anything else, then run section 4,
     steps 3–5. It becomes `complete` again only through step 5's
     verification.
   - Then exit with 67, never READY, never binding a listener. The log says
     `TOMBSTONE` and the kind.
2. **No Tombstone, and the committed head is DEAD.** This is a crash
   between the DEAD commit and the durable Tombstone. Run section 4 from
   step 1.
3. **Otherwise,** normal startup.

`den init` and `pelt init` refuse a Den that holds a Tombstone
(`LIFECYCLE_TOMBSTONE`). Root can delete the directory; nothing claims
otherwise.

## 7. Errors and codes

| Code | When |
|---|---|
| `GOVERNANCE_TERMINAL` | A statement after a DEAD head |
| `LIFECYCLE_DYING` | Any mutation while DYING |
| `LIFECYCLE_TOMBSTONE` | `den init` or `pelt init` on a Den with a Tombstone |
| `RETIRE_CONFIRM_MISMATCH` | `--confirm` is not the node's fingerprint |
| `RETIRE_SAVE_FAILED` | The retirement Tombstone was not durably written; nothing happened |
| exit 67 `TOMBSTONE` | The daemon finished, or found, a Tombstone |

## 8. The claim (decision 8)

This text goes, verbatim, into KNOWN_LIMITATIONS and the operator docs:

> On death or retirement, Werewolf commits the decision, locks the node,
> destroys the node's identity key first and then every file in the Den,
> overwriting each regular file it owns once before removing it, and leaves
> only a minimal Tombstone. It makes no claim about storage media (journals,
> copy-on-write snapshots, SSD wear-levelling), logs, swap, backups or
> copies outside the Den, nor that any governance private key was
> destroyed.

## 9. ARCHITECTURE amendments (decision 3)

These are applied to ARCHITECTURE.md with this spec's approval.

- **§36 Lifecycle model.** The single machine is replaced by the three axes
  of survey 1a:
  - the chain lifecycle `NONE → CEREMONY → ACTIVE → DEAD`, where DEAD is
    terminal;
  - the node condition: Silver, unadmitted, fork-fenced, DYING, TOMBSTONE;
  - governance capability: HEALTHY, DEGRADED, UNRECOVERABLE, an
    observation about key holders that never changes node behaviour.
- **§62 Final death objective.** The first box, "detect unrecoverable
  condition", becomes "key holders determine the condition (a
  legitimate-custody determination); while seven keys remain, they sign a
  death statement; otherwise each node retires locally". The flow
  continues with "commit the terminal decision, lock, destroy the identity
  key, destroy the Den, write the Tombstone". "Anti-forensic cleanup" is
  qualified by the bounded claim of section 8.

## 10. Tests required before merge

**Core:**
- vectors for a death statement from the Stage28 fixed seeds;
- negative tests:
  - a non-empty body;
  - any statement after death, including a second death (`Terminal`);
  - death on a forked head that chains (accepted, L1) and one that doesn't
    (rejected);
- the key check:
  - vectors;
  - a check signature rejected as a statement signature, and the reverse;
  - another challenge or network rejected, and duplicate indices rejected;
  - the verdict thresholds;
- Tombstone:
  - schema round trip;
  - an unknown field, an oversize file or an inconsistent hash rejected.

**Daemon unit tests:**
- the destruction order (the Pelt first);
- the no-follow and link rules: a symlink to a file outside the Den, and a
  hard-linked file, are unlinked and the outside file is unchanged;
- I2: a regular file replaced by a symlink or a hard link between the
  pre-filter and the open is never overwritten (the eligibility comes from
  the descriptor's `fstat`); a FIFO is never opened;
- I3: a stray file placed next to a `complete` Tombstone downgrades it to
  `incomplete` and is destroyed at the next start; an unreadable Den never
  yields `complete`;
- I1: death on a forked node keeps `forked` set and leaves the recorded
  conflict unchanged;
- each `CommitOutcome` at the DEAD commit and at both Tombstone writes,
  using the `local_fs` fault points;
- startup in each state of section 6;
- `den init` and `pelt init` refused.

**Lab gate `tests/integration/death_gate.py`, asserting PASS or FAIL:**
- **Death by file and by sync, on several nodes:**
  - each Den is left with exactly `tombstone.json`, checked by an exact
    allowlist walk;
  - the Tombstone and the Den contain none of a pre-death snapshot's keys,
    fingerprints, addresses, ports, peer names or service names, in raw,
    hex and base64 forms;
  - former peers fail to open a Fang to it or to RPC it;
  - restart exits 67.
- **A node offline during the death** learns from the file later and
  converges.
- **Retirement:** the same assertions, with `kind: "retirement"`.
- **Crash convergence:** an abrupt kill at each step boundary of section 4,
  for both death and retirement. Each run ends in a complete Tombstone, or
  in a locked node marked `incomplete`, never READY.
- **Storage faults:** a read-only Den and a full disk during destruction
  leave `incomplete` and locked, then complete once storage returns.

**Measured and reported, never asserted as erasure:** a node on a
loop-mounted ext4 image is killed, the image unmounted, and the raw image
scanned for the Pelt secret and the Pack keys. The finding is recorded as
"found in N blocks" or "not found on this run".

## Decisions for this spec

**Approved 2026-10-26:**
- L1–L6, exactly as recommended below;
- the additional rule that no destruction begins before the incomplete
  Tombstone is durably committed, for death as well as retirement
  (section 4, step 2);
- three implementation invariants pinned by the owner:
  - **I1.** Death on a fork never resolves that fork (section 1).
  - **I2.** Overwrite eligibility is determined from the safely opened file
    descriptor (section 4, step 4).
  - **I3.** A complete Tombstone may only remain complete if the Den
    exact-allowlist verification succeeds (section 4, step 5; section 6).

The ARCHITECTURE amendments of section 9 are applied.

- **L1. Death on a forked node.** Approved: accept a death statement
  that chains to the node's own head; every other statement stays refused.
  It only narrows, and the alternative leaves a compromised-governance node
  holding live secrets until its operator retires it. The alternative is to
  refuse it and rely on retirement.
- **L2. No node-side confirmation for death.** Approved: none. Death is
  a 7-of-10 decision applied like activation. The alternative is a
  `--confirm-death` flag on `apply`, and sync stopping before a death
  statement, which would make propagation depend on every operator acting.
- **L3. Overwrite policy.** Approved: one pass of zeros, only for
  regular files with one link owned by the daemon's uid; everything else is
  unlinked only. Multiple passes buy nothing on modern media, and the claim
  is bounded anyway.
- **L4. Retirement confirmation.** Approved: `--confirm FINGERPRINT`
  through the running daemon only. An offline retirement (the daemon
  stopped) can come later if needed.
- **L5. Tombstone encoding.** Approved: strict JSON, readable by the
  operator with no tool. The alternative is a binary format: smaller, but
  it buys nothing at under 8 KiB.
- **L6. What a Tombstone start does.** Approved: finish or verify
  destruction, then exit 67 without binding anything. The alternative is
  to stay running and answer `status`, which keeps a process and a control
  socket alive for no benefit.

## Implementation (after approval)

| Step | Scope | State |
|---|---|---|
| 1 | Core: kind 5, `Terminal`, `dead_at`, the key check, the Tombstone type; vectors and negative tests; offline `draft death` and `check new/sign/verify` | Done |
| 2 | Daemon: death apply, the destruction procedure, startup, `den retire`, the error codes, exit 67, the systemd unit; lab gate `death_gate.py` with crash, storage-fault and residue checks | Next |
| 3 | Real-host check on disposable daemons only | |

Step 1 notes:

- **Vectors:**
  - the death core after the Stage29 admission and activation vectors:
    `15a604c4…cbb2`;
  - key-check answer by key 0 on network `77…77`, challenge `33…33`:
    `0f375609…3706`.
- **Daemon guard until step 2.** The daemon applies statements through
  the core, so the step-1 daemon refuses a death statement before any
  effect (`GOVERNANCE_REJECTED`, no archive entry), whether from a file or
  through sync. Startup fails closed on a DEAD head. Step 2 replaces both
  guards with section 4.
- **Key check, whole-check rejection.** `verify` rejects the whole check,
  rather than dropping the answer, when any answer is for another
  challenge or network, has a bad signature or repeats an index, so a
  measurement is never silently smaller than what was presented.
- **The Tombstone type** (`werewolf_core::tombstone`) accepts only its
  canonical bytes. A death Tombstone is checked to contain no governance
  public key in raw, hex or base64 form.
