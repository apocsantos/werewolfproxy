# Stage29 step 3: `governance` RPC wire specification

**Status: specification for review, awaiting owner decisions G1–G4.** No
code exists. This extends the [Stage24 RPC](STAGE24_FINDNODE_RPC_WIRE_SPEC.md)
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
(Stage28). Step 3 makes the archive part of applying: the statement is
written before the head is committed, so every applied statement is
archived; an archive write failure fails the apply (`GOVERNANCE_SAVE_FAILED`)
with no effect. When serving, the archived file must decode, and its hash
must equal the head's recorded hash for that sequence, or the responder
stops there.

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
  with no effect.

## Decisions requested

- **G1. Position by head hash (recommended).** `TG` is the requester's head
  hash rather than a sequence number, so a responder on another network or
  branch returns nothing instead of statements the requester would reject.
- **G2. `K` at most 4 (recommended)**, which keeps every result far below
  `RESULT_LIMIT` with no truncation rule.
- **G3. Archive before commit (recommended).** The archive becomes part of
  applying so that every node can serve what it applied; today it is
  best-effort after commit.
- **G4. Manual sync only for now (recommended).** `governance sync` on
  operator command; automatic periodic sync (5c) stays a follow-up.
