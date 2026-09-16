# Stage 10: connection-bound OPEN authorization

Stage 9 base: `23ae82c633990d0d594539f738feaf8569f489fe`.
Branch: `codex/stage10-replay-hardening`. No Stage 11 changes.

## Security contract

A signed OPEN captured on one connection cannot authorize a target on another
connection. TCP uses a fresh receiver challenge; QUIC uses the completed TLS
connection's exporter. Restart, crash, and runtime-state disposal do not restore
validity of captured requests. Security assumes sound OS randomness, TLS, and
signature primitives. This does not prevent a live TCP handshake relay to the
intended receiver, or an authenticated peer signing new permitted requests.

No persistent replay state is introduced. No legacy protocol probing, downgrade,
or compatibility flag is introduced. Old binaries retain their historical behavior.

## Exact encoding and messages

Each message is a UTF-8 JSON object followed by LF. Object key order is immaterial.
All shown fields are mandatory. Unknown/duplicate fields, missing fields, wrong
types, trailing non-whitespace JSON data, and oversized lines are rejected.
`SF`/`RF` are canonical Pelt fingerprints. `SP`/`RP` and `CX`/`SX` are canonical
standard padded base64 encodings of 32-byte Ed25519 and X25519 public keys;
`SIG` encodes a 64-byte Ed25519 signature. `N` is 32 lowercase hex characters
encoding 16 OS-CSPRNG bytes. `C` is 64 lowercase hex characters encoding 32
OS-CSPRNG bytes. `R` is the exact requested remote string, nonempty, at most
512 UTF-8 bytes, without whitespace/control characters. No target normalization
is performed for signing; Stage 9 separately parses and authorizes it.

TCP receiver challenge:
```json
{"cmd":"fang.tcp.challenge","protocol":"fang-tcp-v3","receiver_fingerprint":"RF","challenge":"C"}
```
TCP OPEN:
```json
{"cmd":"fang.pipe","protocol":"fang-tcp-v3","sender_pubkey":"SP","sender_fingerprint":"SF","receiver_fingerprint":"RF","challenge":"C","nonce":"N","client_x25519":"CX","remote":"R","signature":"SIG"}
```
TCP ACK:
```json
{"cmd":"fang.tcp.ack","ok":true,"protocol":"fang-tcp-v3","session":"fang-v3-secure","receiver_pubkey":"RP","receiver_fingerprint":"RF","sender_fingerprint":"SF","challenge":"C","nonce":"N","client_x25519":"CX","server_x25519":"SX","remote":"R","signature":"SIG"}
```
QUIC OPEN:
```json
{"cmd":"fang.quic.open","protocol":"fang-quic-v3","sender_fingerprint":"SF","receiver_fingerprint":"RF","nonce":"N","remote":"R","signature":"SIG"}
```
QUIC ACK:
```json
{"cmd":"fang.quic.ack","ok":true,"protocol":"fang-quic-v3","receiver_pubkey":"RP","receiver_fingerprint":"RF","sender_fingerprint":"SF","nonce":"N","remote":"R","signature":"SIG"}
```

## Exact signed transcripts

`LP(x) = uint32_be(byte_length(x)) || x`.
`T(domain; fields...) = ASCII("WWP-HS") || 0x00 || LP(ASCII(domain)) || LP(field1) ...`.
Text fields below are UTF-8; `raw(X)` means decoded bytes. `0x01` is one byte,
not the string `true`. Retained OPEN means the complete transcript bytes, not
JSON or the OPEN signature.

```
TCP_OPEN = T("fang.tcp.open/v3";
  "fang-tcp-v3", SF, raw(SP), RF, raw(C), raw(N), raw(CX), R)
TCP_ACK = T("fang.tcp.ack/v3";
  "fang-tcp-v3", 0x01, "fang-v3-secure", TCP_OPEN, raw(RP), raw(SX))
QUIC_OPEN = T("fang.quic.open/v3";
  "fang-quic-v3", SF, RF, raw(N), R, B)
QUIC_ACK = T("fang.quic.ack/v3";
  "fang-quic-v3", 0x01, QUIC_OPEN, raw(RP))
```

ACK verification additionally checks command, protocol, success, all echoed
identity/request fields, TCP session domain and echoed client ephemeral key.
The receiver public key must derive the expected receiver fingerprint. Therefore
an ACK for a different OPEN/challenge/exporter connection cannot substitute for
the retained request even with otherwise plausible echo fields.

## QUIC exporter

Locked Quinn 0.11.9 exposes
`Connection::export_keying_material(&mut output, label, context)` through its
TLS session. It uses rustls exporter semantics from the completed handshake.
Both endpoints derive identical bytes for the same label/context/connection;
independent TLS connections derive different material. OPEN never uses 0-RTT.

```
E = export_keying_material(
  length = 32,
  label = ASCII("EXPORTER-WerewolfProxy-Fang-QUIC-v3"),
  context = ASCII("werewolfproxy/fang-quic-v3"))
B = BLAKE3(ASCII("werewolfproxy/fang-quic-v3/channel-binding") || 0x00 || E)
```

`B` is the full 32-byte digest, a PUBLIC channel-binding value, not a bearer
credential or authentication substitute. Security does not depend on its secrecy.
`E` is never transmitted, logged, or persisted. Neither value is a JSON field.
Exporter failure rejects the connection. Signature verification uses locally
derived `B`; TLS certificate verification behavior remains unchanged.

## Ordering and errors

TCP obtains global admission, freezes receiver identity/key material, generates
and sends the challenge, then reads one bounded OPEN. It validates field encodings,
Pack membership and key identity, receiver/challenge binding, signature, and
challenge deadline, acquires authenticated peer admission, and consumes the
challenge. It performs Stage 9 authorization, connects only to the authorized
SocketAddr set, signs ACK with the frozen receiver identity, then proxies using
the existing encrypted framing/KDF. Concurrent `pelt.init` cannot change the
identity used by this handshake. A connection never returns to OPEN parsing.

QUIC obtains bounded context/TLS admission, completes TLS, derives B, and accepts
bounded OPEN streams. Parsing, Pack-key lookup, receiver binding and signature
verification precede peer admission and atomic replay reservation. Reservation
precedes Stage 9 authorization and target connection. Successful signed ACK is
sent only after target connection. One stream accepts one OPEN; following bytes
are application data, not another authorization request.

Rejections before authorization/connect cause zero target connections and no
successful ACK. A failure while delivering ACK can occur after an authorized
target has already connected; this is not an authorization rejection.
TCP closes without a detailed error response. QUIC uses named private constant
`V3_REJECT_CODE = 0x575703`, empty connection-close reason or generic stream
reset/stop. Repository/history review found no previous production application
code assignment; test code 0 denotes orderly closure. Pre-TLS capacity rejection
uses Quinn's connection refusal. No remote policy, Pack, signature, DNS, replay,
receiver-binding or capacity reason is disclosed in an application error message.
Local handshake diagnostics are generic. Timing/transport-level refusal differences
are not claimed to be indistinguishable.

## Admission and replay bounds

| Limit | Value | Lifetime/exhaustion |
|---|---:|---|
| Global handshake permits, shared TCP/QUIC | 256 | Handshake only; reject without waiting |
| Authenticated peer handshakes | 16 | Signature verified before claiming quota; released on drop |
| Active QUIC contexts | 1024 | Held through connection closure and child-task cleanup |
| Concurrent OPEN handshakes per QUIC connection | 8 | Released after handshake; exhaustion closes connection |
| Replay reservations per QUIC connection | 256 | Retained until closure; exhaustion rejects OPEN |

Peer counters exist only while holding global admission, bounding counter-map
cardinality. At most 1024 * 256 = 262144 replay reservations exist; no extra global
retained-state quota is needed. These limits bound handshake/replay state, not all
application proxy traffic or all memory internal to TLS/QUIC/kernel networking.

TCP challenge state is connection-local and consumed once, without a shared nonce
cache. QUIC reserves typed `(authenticated sender [u8;8], nonce [u8;16])` keys
under a mutex in a connection-owned `QuicReplayV3`. Protocol/domain and connection
identity are structural, not ambiguous shared String keys. The membership check
and insertion are atomic. Entries are retained after policy denial, connect/ACK
failure and stream closure; there is no TTL/LRU/pruning in an active connection.
Invalid signatures never reserve entries or victim peer quota. Valid authenticated
traffic can exhaust a connection's allowance, but cannot evict protection. A new
connection has a fresh binding and may start new requests safely.

## Resource/time bounds

| Operation | Bound |
|---|---:|
| Challenge JSON line including LF | 512 bytes |
| OPEN or ACK JSON line including LF | 4096 bytes |
| Receiver OPEN / initiator challenge read | 5 seconds |
| Initiator ACK read | Remaining portion of 15-second initiator handshake deadline |
| TCP challenge lifetime | 5 seconds, monotonic, additionally capped from acceptance |
| Individual handshake write | 1 second, capped by enclosing deadline |
| Receiver handshake | 11 seconds overall |
| Initiator handshake after peer connection | 15 seconds overall |
| TCP connect / QUIC TLS establishment | 5 seconds |
| First QUIC stream arrival after TLS | 5 seconds |
| Target resolution/authorization and all address attempts together | 5 seconds |

Reads consume only through LF, with no unbounded read_line or discarded prefetched
application bytes. OS RNG failure fails closed. Nonce uniqueness does not depend
on the wall clock. Tokio deadlines are cooperative; an OS DNS task can finish in
the background after the request stops waiting. Established QUIC connections
remain subject to existing transport idle behavior; no replay-expiry clock exists.

## Validation and compatibility

`replay_v3_tests` uses isolated loopback listeners with accepted-target counters.
Process tests build the immutable Stage 9 commit and current daemon offline into
separate target directories. Private temporary homes contain explicit Stage 9
deny-by-default grants and are removed by the fixtures. Actual SIGTERM restart
and SIGKILL/restart preserve Pelt/Pack/policy bytes; captured TCP/QUIC OPENs fail
without increasing target counters, while fresh requests succeed.

| Property | Evidence |
|---|---|
| TCP valid, random canonical nonce, unique/expired/one-shot challenge | Unit and live receiver tests |
| Frozen receiver identity | In-memory mutation and actual pelt.init after challenge; old identity verifies ACK |
| TCP immediate/concurrent/new-connection replay | Captured OPEN rejected; target counters unchanged |
| Wrong TCP challenge/receiver/target/ephemeral/signature | Live signed-request mutation matrix |
| TCP ACK substitution | Transcript/echo and cryptographic signature unit tests |
| QUIC exporter equality/separation | Real completed client/server TLS connections |
| QUIC same stream, another stream, concurrent duplicate, new connection | Live probes; one authorization per stream and atomic reservation |
| QUIC wrong connection/receiver/target/signature | Live probes and fake receiver ACK substitution |
| Restart/crash replay | Actual daemons, both transports, fresh positive controls |
| No unsafe eviction | Fill all 256 reservations after denied authorization; neither old nor extra request accepted |
| Admission | Unit exhaustion/release for all five bounds; live global/peer/open exhaustion and spoofed identity rejection |
| Parsing | Live OPEN unknown/duplicate/missing/type/trailing/oversized/noncanonical cases on both transports |
| Slow/fragmented/buffer preservation | Live expiry/incomplete QUIC sender and fragmented TCP; bytewise reader fixture preserves tail |
| Stage 9 rejection properties | Existing 36 TCP/QUIC IPv4/IPv6 authorization cases migrated to v3 |

| Initiator -> receiver | TCP and QUIC result |
|---|---|
| Old -> old | Existing v2 behavior, tested with actual Stage 9 binary |
| Old -> new | Fail closed |
| New -> old | Fail closed with bounded timeout |
| New -> new | v3 success |

The isolated acceptance lab retains deny-by-default plus explicit grants. Its
checks cover QUIC, encrypted TCP, plain TCP, ordered fallback, QUIC restoration,
50 MiB encrypted transfer with SHA256, binary raw echo across all transports,
and profile persistence/restart. Historical shell/lab selector text
`tcp-encrypted-v2` is preserved: it now selects TCP v3 handshake with unchanged
encrypted application framing. It does not enable handshake downgrade.

## Scope

New private daemon modules: handshake and admission. Transport helpers are visible
only within the daemon; there is no public library API expansion. No dependency
or Cargo lockfile changes from Stage 9. Existing Ed25519/X25519 primitives, BLAKE3
session KDF, ChaCha20-Poly1305 framing, identity pinning, Pack/Pelt semantics,
Stage 9 target policy, Fang lifecycle, shell fallback ordering, plain TCP,
persistence formats and TLS SkipServerVerification remain unchanged.

Finite admission can be exhausted by hostile traffic, resulting in fail-closed
availability loss. Replay protection is cryptographic/probabilistic, not a proof
of impossible random collision. TCP ACK substitution uses executable transcript
fixtures rather than a separate fake-server transport test. Context-limit
exhaustion is tested through the production admission type rather than opening
1025 simultaneous live TLS sessions. No power-loss hardware test is claimed;
SIGKILL exercises abrupt process-state loss. No replay persistence is needed.
