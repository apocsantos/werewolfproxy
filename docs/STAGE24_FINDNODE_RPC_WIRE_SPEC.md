# Stage24 step 2: FindNode/Ping RPC wire specification

**Status: approved by the owner 2026-10-19; implementation in progress.**
This is the first
extension of the frozen [v1 wire contract](V1_WIRE_CONTRACT.md). It follows
the accepted [Stage24 design](STAGE24_FINDNODE_DESIGN.md) (transport option
A, Pack-only queries) and reuses the [Stage10](STAGE10_REPLAY_HARDENING.md)
notation, encodings and authentication. Existing OPEN, ACK and challenge
messages, transcripts, KDF and framing are unchanged byte for byte.

## Scope

Two read-only operations between Pack members:

- `ping`: returns the responder's own signed contact record, if it has one.
- `find_node`: returns up to `k` verified contact records closest (XOR) to a
  target `NodeId`.

An RPC never opens a target connection, never takes a Stage11C authority
lease, never changes Pack, Target Policy or any persisted state, and never
grants trust. Records are `werewolf_core::contact::ContactRecord` (step 1):
self-signed, at most 7 days valid, literal `quic://`/`tcp://` addresses.

## Where the RPC runs

| Transport | Position | Requests per carrier |
|---|---|---|
| QUIC | First line of a client-opened bidirectional stream, in place of QUIC OPEN | One per stream; several streams per connection, within the existing per-connection limits |
| Encrypted TCP | The line after the receiver's unchanged TCP challenge, in place of TCP OPEN | One per TCP connection; the challenge is single-use |

The receiver dispatches on the `cmd` value of that first line. Both
transports are supported so discovery keeps working where UDP is blocked.
QUIC 0-RTT is never accepted for an RPC, as for OPEN.

## Encodings

As Stage10: UTF-8 JSON object plus LF; all shown fields mandatory; unknown,
duplicate or missing fields, wrong types, trailing data and oversized lines
are rejected. `SF`, `RF`, `SP`, `RP`, `N`, `C`, `SIG`, `B` are as in Stage10.
New values:

- `OP`: exactly `"ping"` or `"find_node"`.
- `TG`: for `find_node`, 64 lowercase hex characters (a 32-byte `NodeId`);
  for `ping`, the empty string.
- `K`: JSON integer; `find_node` 1..=20, `ping` exactly 0.
- `REC`: standard padded base64 of `ContactRecord::encode()` bytes.

## Messages

QUIC RPC (client to receiver):
```json
{"cmd":"fang.quic.rpc","protocol":"fang-quic-v3","sender_fingerprint":"SF","receiver_fingerprint":"RF","nonce":"N","op":"OP","target":"TG","k":K,"signature":"SIG"}
```
QUIC RPC result (receiver to client):
```json
{"cmd":"fang.quic.rpc.result","ok":true,"protocol":"fang-quic-v3","receiver_pubkey":"RP","receiver_fingerprint":"RF","sender_fingerprint":"SF","nonce":"N","op":"OP","records":["REC",...],"signature":"SIG"}
```
TCP RPC (client to receiver, after the unchanged challenge):
```json
{"cmd":"fang.tcp.rpc","protocol":"fang-tcp-v3","sender_pubkey":"SP","sender_fingerprint":"SF","receiver_fingerprint":"RF","challenge":"C","nonce":"N","op":"OP","target":"TG","k":K,"signature":"SIG"}
```
TCP RPC result (receiver to client):
```json
{"cmd":"fang.tcp.rpc.result","ok":true,"protocol":"fang-tcp-v3","receiver_pubkey":"RP","receiver_fingerprint":"RF","sender_fingerprint":"SF","challenge":"C","nonce":"N","op":"OP","records":["REC",...],"signature":"SIG"}
```

The TCP RPC carries no X25519 key: the result travels inside the
already-authenticated TLS 1.3 channel and no encrypted Fang framing follows.
There is no negative result message; every failure is a Stage10 rejection.

## Signed transcripts

With Stage10 `T`, `LP` and `raw()`; `u8(K)` is one byte; `raw(TG)` is 32
bytes for `find_node` and empty for `ping`:

```
QUIC_RPC = T("fang.quic.rpc/v1";
  "fang-quic-v3", SF, RF, raw(N), OP, raw(TG), u8(K), B)
QUIC_RPC_RESULT = T("fang.quic.rpc.result/v1";
  "fang-quic-v3", 0x01, QUIC_RPC, raw(RP), u8(count), raw(REC_1), ..., raw(REC_count))

TCP_RPC = T("fang.tcp.rpc/v1";
  "fang-tcp-v3", SF, raw(SP), RF, raw(C), raw(N), OP, raw(TG), u8(K))
TCP_RPC_RESULT = T("fang.tcp.rpc.result/v1";
  "fang-tcp-v3", 0x01, TCP_RPC, raw(RP), u8(count), raw(REC_1), ..., raw(REC_count))
```

The `/v1` domains are new and differ from every OPEN/ACK domain, so no RPC
signature verifies as an OPEN or ACK and vice versa. QUIC binds to the
connection exporter `B`; TCP binds to the fresh single-use challenge `C`, the
same freshness Stage10 gives OPEN. A result signs the complete retained
request transcript, so a result cannot be moved to another request.

## Receiver rules

In order, mirroring Stage10 OPEN handling:

1. Global admission and TLS as today (unchanged). Silver locked: reject.
2. Read one bounded line (`MESSAGE_LIMIT`, 4096 bytes); dispatch on `cmd`.
3. Validate encodings, `op`/`target`/`k` combinations and receiver binding.
4. Sender must be a current Pack member: QUIC uses the Pack key for `SF`;
   TCP requires `SP` to derive `SF` and equal the Pack key. Verify the
   signature, then claim authenticated peer admission.
5. Replay: QUIC reserves `(SF, N)` in the connection's existing replay set,
   shared with OPEN; TCP consumes the challenge.
6. Per-peer RPC rate limit (new): token bucket per Pack fingerprint,
   burst 16, refill 4 per second. Exhausted: reject.
7. Build records: `ping` returns the responder's own current record, or none
   if it advertises no address; `find_node` returns at most `K` records from
   its verified contact set, ordered by XOR distance to `TG`, excluding the
   requester's own key, each re-verified against the local clock.
8. Drop the farthest records until the result line fits `RESULT_LIMIT`
   (new: 32768 bytes); sign; send; QUIC finishes the stream, TCP sends TLS
   `close_notify` and closes.

No step creates a target connection or authority lease. Rejections use the
existing generic paths (TCP close; QUIC `V3_REJECT_CODE` reset/stop).

## Requester rules

1. Read the result with limit `RESULT_LIMIT` within `CLIENT_WINDOW`.
2. Check `cmd`, `protocol`, `ok`, and that every echoed field equals the
   request; `RP` must derive the expected receiver fingerprint (already
   pinned by TLS). Verify the result signature over the retained request.
3. `count` ≤ `K` for `find_node`; ≤ 1 for `ping`, whose record key must equal
   `RP`.
4. Decode and `verify` each record with the local clock. Invalid or expired
   records are discarded individually (clock skew is expected); a malformed
   result envelope rejects the whole result.
5. Never add a discovered node to the Pack or Target Policy.

## Compatibility and opacity

- A pre-Stage24 receiver deserializes the first line as OPEN with unknown
  fields denied, so an RPC fails exactly like any malformed OPEN. Nothing on
  the wire advertises RPC support; a requester treats rejection as "no
  answer", with no fallback or downgrade.
- A Stage24 receiver handles OPEN identically to today; dispatch only adds
  two `cmd` values.
- New observable traffic: short request/response exchanges and result sizes
  that depend on record count. Contents stay inside TLS. This is accepted for
  the closed overlay and noted for Stage13-style morphology review.
- A responder reveals the contacts it knows to its Pack members. Accepted for
  Phase 1 (Pack-only queriers).

## Limits

| Limit | Value | Notes |
|---|---:|---|
| Request line | 4096 bytes | Existing `MESSAGE_LIMIT` |
| Result line | 32768 bytes | New `RESULT_LIMIT`; responder truncates by distance |
| `k` | 1..=20 | `lookup::MAX_K` |
| Records per `ping` result | 0..=1 | Own record only |
| Per-peer RPC rate | burst 16, 4/s | New; in-memory, per Pack fingerprint |
| Deadlines | `READ_WINDOW`, `SERVER_WINDOW`, `CLIENT_WINDOW` | Existing |
| QUIC concurrency/replay | 8 concurrent, 256 reservations per connection | Existing, shared with OPEN |

## Test plan

- Transcript vectors for all four new domains; cross-domain rejection (RPC
  signature presented as OPEN and the reverse).
- Receiver: non-Pack sender, wrong key, wrong receiver, bad `op`/`target`/`k`
  combinations, replayed nonce (QUIC), reused challenge (TCP), Silver locked,
  rate limit, oversized request; each rejects with no target connection.
- Result: tampered echo fields, wrong receiver key, `count` > `K`, oversized
  line, foreign record in `ping`, expired and forged records discarded
  individually.
- OPEN regression: the existing Stage10/12 suites unchanged and passing.
- Compatibility: an RPC sent to the pre-Stage24 receiver is rejected like a
  malformed OPEN.
- Lab: three wolves (A knows B, B knows C); A pings B and finds C's record
  through B over QUIC and encrypted TCP; A still cannot open a Fang to C.

## Decisions (approved 2026-10-19)

1. The messages and transcripts above are the v1 contract extension.
2. RPC runs over both QUIC and encrypted TCP in Phase 1.
3. Limits: `RESULT_LIMIT` 32 KiB; per-peer rate burst 16, refill 4 per second.
4. Results are signed, consistent with ACK.
