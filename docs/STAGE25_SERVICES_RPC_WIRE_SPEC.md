# Stage25 step 2: `services` RPC wire specification

**Status: proposal, awaiting owner approval.** No wire code lands before
approval. This extends the [Stage24 RPC](STAGE24_FINDNODE_RPC_WIRE_SPEC.md)
as decided in the [Stage25 design](STAGE25_SERVICE_DISCOVERY_DESIGN.md)
(decisions 1, 2 and 4). It adds one `op` value. Commands, message fields,
transcript domains, signatures, OPEN, ACK, challenge, KDF and framing are
unchanged byte for byte.

## Scope

One read-only operation between Pack members:

- `services`: returns the responder's own published services that the
  responder's Target Policy would let the requester open right now.

Like `ping` and `find_node`, it never opens a target connection, never
takes a Stage11C authority lease, never changes any persisted state and
never grants trust.

## Encodings

As Stage24. Changed values:

- `OP`: exactly `"ping"`, `"find_node"` or `"services"`.
- `TG`: for `services`, the empty string (as `ping`).
- `K`: for `services`, exactly 0 (as `ping`).
- `REC`: in a `services` result, standard padded base64 of
  `werewolf_core::published::PublishedService::encode()` bytes: one kind
  byte (`tcp` 0, `http` 1, `https` 2, `ssh` 3), then the name and the target,
  each as a `u16` big-endian length and UTF-8 bytes. Names are 1–128 bytes,
  canonical and free of control characters; targets are canonical literal
  `IP:port` of at most 47 bytes. One entry is at most 180 bytes.

## Messages and transcripts

The four Stage24 messages and their transcripts are used unchanged:

```
QUIC_RPC = T("fang.quic.rpc/v1"; "fang-quic-v3", SF, RF, raw(N), "services", raw(""), u8(0), B)
QUIC_RPC_RESULT = T("fang.quic.rpc.result/v1";
  "fang-quic-v3", 0x01, QUIC_RPC, raw(RP), u8(count), raw(REC_1), ..., raw(REC_count))
```

and the same for TCP with its challenge `C`. `OP` is signed in the request
transcript, and the result signs the complete retained request, so a
`services` result cannot be presented as a `ping` or `find_node` result or
the reverse, and its entries cannot be moved to another request. No new
domain is needed.

## Receiver rules

Stage24 receiver steps 1–6 (admission, TLS, Silver, bounded line, field
validation, Pack-member sender and signature, replay, per-peer rate limit)
apply unchanged; `services` shares the per-peer RPC token bucket. Step 7
gains one case:

7. `services`: take the responder's published list (validated when loaded
   and on every change; at most 32). Keep an entry only if
   `target_policy::authorize(policy, requester_fingerprint, target)` would
   succeed now: under `Grants`, the requester's fingerprint has the exact
   target; under `LegacyAllow`, every entry; under `Deny` or `Invalid`, none.
   This is the same decision OPEN applies to that requester and target.
   Published targets are literal, so the check never resolves DNS. Keep the
   published order.

Step 8 is unchanged, except that a `services` result is never truncated: 32
entries of at most 180 bytes encode to at most 7,680 base64 bytes, well
inside `RESULT_LIMIT` (32 KiB). If a result would still not fit, the
receiver rejects instead of dropping entries.

## Requester rules

Stage24 requester steps 1–2 (deadline, envelope, echoed fields, receiver key,
result signature over the retained request) apply unchanged. Then:

3. `count` ≤ 32.
4. Every `REC` must be canonical base64 (as Stage24) and decode with
   `PublishedService::decode`, which applies the length, UTF-8, name and
   target rules. Entry names must be unique within the result. **Any bad
   entry rejects the whole result**: unlike contact records, entries are not
   signed individually, so a malformed entry means the responder itself is
   faulty or hostile, and nothing from that answer is shown.
5. Never write a received entry to the Pack, Target Policy or Service
   Registry, and keep no cache (design decision 3). `service add --from`
   (step 3) copies from a fresh answer only.

## Compatibility and opacity

- A Stage24 receiver without `services` rejects the unknown `op` in
  `RpcRequest::parse` like any invalid RPC; a pre-Stage24 receiver rejects
  it like any malformed OPEN. The requester reports `rejected`, with no
  fallback.
- An empty answer does not distinguish "nothing published" from "nothing
  granted to you" (design decision 2).
- New observable traffic: one more short request/response kind whose result
  size depends on the number of entries visible to that requester. Contents
  stay inside TLS; noted for Stage13-style morphology review, as Stage24.

## Limits

| Limit | Value | Notes |
|---|---:|---|
| Entries per `services` result | 0..=32 | `published::MAX_PUBLISHED` |
| Encoded entry | ≤ 180 bytes | `published::MAX_ENCODED_LEN` |
| Result line | 32768 bytes | Existing `RESULT_LIMIT`; never reached by `services` |
| Per-peer RPC rate | burst 16, 4/s | Existing, shared with `ping` and `find_node` |
| Deadlines, QUIC concurrency and replay | — | Existing, unchanged |

## Test plan

- Transcript vectors: a `services` request and result on both transports.
  The result vector carries two entries. Cross-op rejection: a `services`
  result checked against the retained `ping` request, and the reverse.
- Receiver: `TG` non-empty or `K` ≠ 0 rejected. Visibility under `Grants`
  shows only the requester's granted targets, and two requesters see
  different subsets. `LegacyAllow` shows all; `Deny` and `Invalid` show none.
  A grant removed between two requests hides the entry on the second. Silver
  locked rejects. The rate limit is shared with the other RPCs. No target
  connection or authority lease is created.
- Requester: `count` > 32, a non-canonical base64 entry, an undecodable
  entry, a control character in a name, a hostname target and duplicate
  names each reject the whole result. Tampered echoed fields or a signature
  over another request are rejected.
- Compatibility: `services` sent to a receiver that only knows `ping` and
  `find_node` is rejected; OPEN, `ping` and `find_node` regression suites are
  unchanged and passing.

## Decisions needed

1. Approve the messages and rules above as the v1 contract extension (one
   new `op` value, no new domains).
2. A bad entry rejects the **whole** result (recommended), rather than
   dropping that entry as `find_node` does with bad records.
3. Entries keep the provider's published order (recommended), rather than
   being sorted by name.
