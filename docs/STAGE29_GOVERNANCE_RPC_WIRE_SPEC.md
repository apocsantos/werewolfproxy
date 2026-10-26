# Stage29 step 3: `governance` RPC wire specification

**Status: approved 2026-10-26 (G1–G4, with the owner's G3 condition on the
crash window; see [Decisions](#decisions-approved-2026-10-26)); implemented in
step 3, see [Implementation](#implementation).** This extends the [Stage24 RPC](STAGE24_FINDNODE_RPC_WIRE_SPEC.md)
as decided in the [Stage29 design](STAGE29_ACTIVATION_DESIGN.md) (decision
5a, addition D). It adds one `op` value. Commands, message fields,
transcript domains, signatures, OPEN, ACK, challenge, KDF and framing are
unchanged byte for byte.

## Scope

One read-only operation between Pack members:

- `governance`: given the requester's current governance head hash, the
  responder returns the statements that follow it in the responder's own
  chain, up to `K`.

Like `ping`, `find_node` and `services`, it never opens a target connection,
never takes a Stage11C authority lease, never changes any persisted state on
the responder and never grants trust. Its results carry no authority of
their own: a statement is applied only if it verifies with seven governance
signatures against the requester's head, through the same `apply` path as a
file. A responder can withhold statements; it can never forge one.

## Encodings

As Stage24. Changed values:

- `OP`: exactly `"ping"`, `"find_node"`, `"services"` or `"governance"`.
- `TG`: for `governance`, 64 lowercase hex characters: the requester's
  current head hash (`raw(TG)` is 32 bytes, as for `find_node`).
- `K`: for `governance`, 1..=4.
- `REC`: in a `governance` result, standard padded base64 of a Stage28/29
  statement encoding (`Statement::encode()`), in sequence order.

The largest valid statement is 2,777 bytes (3,704 base64 characters), so
four fit well inside `RESULT_LIMIT` (32 KiB).

## Messages and transcripts

The Stage24 messages and transcripts are used unchanged; `OP` and `TG` are
signed in the request, and the result signs the complete retained request.
A `governance` result therefore cannot be presented as another op's result,
or for another head hash. No new domain is needed.

## Receiver rules

Stage24 receiver steps 1–6 apply unchanged (TLS and admission, a Silver-
**or admission-** locked authority rejects, bounded line, field validation,
Pack-member sender and signature, replay, the shared per-peer RPC rate
limit). Step 7 gains one case:

7. `governance`:
   - If the responder has no governance head, or `TG` is not the hash of a
     statement it has applied, return zero records. This covers another
     network, a fork or an unknown head, and the responder reveals nothing
     about why.
   - Otherwise return its applied statements with sequences `s+1 …
     min(s+K, head)`, where `s` is the sequence at which `TG` was applied,
     read from its statement archive.
   - Stop at the first missing archive entry; a gap is never skipped.

A responder that is itself forked still answers from its applied chain; the
requester judges. Step 8 is unchanged; a `governance` result is never
truncated (at most four statements fit).

## Requester rules

Stage24 requester steps 1–2 apply unchanged (deadline, echoed fields,
receiver key, result signature). Then:

3. `count` ≤ `K`.
4. Every `REC` is canonical base64 and decodes with `Statement::decode`; a
   malformed entry rejects the whole result (the statements form a chain,
   so a hole in it is not usable).
5. The statements must form a chain from `TG`: the first names `TG` as
   `previous`, each next names the previous one's hash, and sequences
   increase by one. A result that does not rejects as a whole.

Statements are never applied from one result alone; see `governance sync`.

## `governance sync` (addition D)

```text
werewolfctl governance sync [--peer NAME] [--transport quic|tcp]
```

1. **Ask.** Ask every Pack member (or only `--peer`), at most three at a
   time, within seven seconds. A routed member is asked through its relay
   (Stage26 decision 5b), and each member is asked once per round with
   `TG` = the current head hash and `K` = 4.
2. **Collect.** For the next sequence `s+1`, collect the distinct
   statements (by statement hash) offered as that sequence across all
   valid results.
3. **Verify each candidate independently** against the current head
   (`governance::apply`, as for a file); discard candidates that fail.
4. **Decide.**
   - **No valid candidate:** stop.
   - **More than one distinct valid candidate:** conflicting valid
     successors. Record the fork durably (the head's `forked` flag, exactly
     as a conflict with an applied statement) **before applying either**,
     and stop with `GOVERNANCE_FORK`.
   - **Exactly one:** apply it through the same daemon path as `governance
     apply`, with all its effects, persisted before published. Then repeat
     from step 2 for the next sequence, using the same results (their later
     statements), and start a new round when they are used up.
5. **Report** per peer: `ok` with the number of statements offered,
   `rejected`, `unreachable`, `timeout`, `bad_route`, `not_queried`; and the
   statements applied (sequence and hash).

A peer whose result is malformed or does not chain is reported `rejected`
and contributes no candidate. Sync works whether or not this node is
admitted. Once a network is activated, though, an unadmitted node is not
in its peers' Packs, so they refuse its RPCs: an unadmitted node receives
its admission by `governance apply`, as before.

Sync is refused under Silver (`SILVER_ACTIVE`), without a Pelt (`NO_PELT`),
without a governance head (`GOVERNANCE_NO_GENESIS`; a genesis is only ever
applied from a file with `--expect-hash`) and while another sync or
discovery runs (`DISCOVER_BUSY`).

## Responder storage

The responder serves statements from its archive `governance_NNNNNNNN.wwgov`
(Stage28). Step 3 makes the archive part of the logical apply transaction:
the statement is written before the head is committed, so every applied
statement is archived; an archive write failure fails the apply
(`GOVERNANCE_SAVE_FAILED`) with no effect. When serving, the archived file
must decode, and its hash must equal the head's recorded hash for that
sequence, or the responder stops there.

**Crash window (owner condition on G3).** `archive persisted → crash → head
not committed` leaves an archive entry beyond the committed head. The rule:

- Archive entries beyond the committed head are **uncommitted artefacts**,
  never applied history. They are never served (serving checks the head's
  recorded hash) and never consulted by `apply`.
- On apply of sequence `n`, an existing archive entry for `n` is reused if
  it is byte-identical to the statement being applied; otherwise it is
  atomically replaced, before the head is committed. An orphan therefore
  never blocks a legitimate apply and is never mistaken for history.
- Replacement uses the Den's atomic replace (write a temporary file, then
  rename it), so the entry for `n` is always either the old artefact or the
  new statement, never a mixture.

This makes the state recoverable and the apply idempotent across a crash at
any point.

## Limits

| Value | Limit |
|---|---|
| `K` | 1..=4 |
| Records per result | ≤ `K` |
| Sync concurrency, budget | 3 peers at a time, 7 seconds per round |
| Rounds per sync | at most 16 (64 statements) |
| Rate limit | the shared per-peer RPC bucket (burst 16, 4 per second) |

## Test plan

- Transcript: the `governance` op and `TG` are signed; a result for another
  `TG` or another op fails.
- Receiver:
  - an unknown `TG`, another network, or no head returns zero records;
  - `K` bounds;
  - a gap in the archive stops the result;
  - a locked authority (Silver or unadmitted) rejects;
  - a non-member is rejected;
  - the shared rate limit applies.
- Requester:
  - `count` > `K`;
  - a malformed entry;
  - a result that does not chain from `TG`;
  - a valid chain applies in order.
- Sync (addition D):
  - two peers offering the same successor apply it once;
  - two peers offering different valid successors fence before either
    applies;
  - an invalid candidate is ignored while a valid one applies;
  - a revocation reaches a node through a peer and removes the member;
  - a routed member is asked through its relay.
- Archive: written before commit; a failed archive write fails the apply
  with no effect; an orphan entry beyond the head is never served, is
  reused when identical and replaced when different, and a crash between
  archive and commit recovers on the next apply.

## Decisions (approved 2026-10-26)

- **G1. Position by head hash.** The hash identifies both position and
  history: a peer on another network, another branch or without that
  ancestry returns zero records. It reveals no unnecessary state and never
  makes a sequence number a false sign of compatibility between chains.
- **G2. `K` at most 4.** Four statements fit comfortably in `RESULT_LIMIT`
  even at their maximum size, so there is no truncation rule: `count ≤ K`.
- **G3. Archive before commit**, as part of the logical apply transaction,
  with the crash-window rule above: archive entries beyond the committed
  head are uncommitted artefacts, reused on apply if byte-identical and
  otherwise atomically replaced before commit.
- **G4. Manual sync only.** Wire protocol, archive semantics, fork
  detection and manual convergence come first; automatic sync would make
  governance propagation a permanent property of the system rather than a
  deliberate action, and stays a follow-up.

## Implementation

Done in step 3:

- the `governance` op in the RPC layer (`RpcRequest::Governance`, results
  verified to chain from `TG`);
- the responder serving from the archive, checked against the head's
  recorded hashes;
- `governance sync`, resolved outside mutation admission like `service add
  --from` and applied through the internal `governance.sync_apply`
  mutation, which re-verifies every statement, and any conflict, against
  the live head;
- the archive written before the commit, through the Den's atomic replace.

Tests:

- RPC unit tests for the op, its fields, chaining and cross-op results;
- a unit test that a statement which cannot be archived has no effect;
- the lab gate `tests/integration/sync_gate.py`.

