# Stage 16: pre-auth and resource-exhaustion hardening

## Scope

Stage 16 bounds remote-triggered work before sender authentication and during authenticated session establishment. It does not change a Stage10 transcript, KDF, AEAD framing, TLS receiver authentication, Pack/Pelt trust, target authorization rules, authority semantics, fallback policy, persistent state, or traffic morphology.

STAGE16_WIRE_FORMAT_CHANGE = NONE for valid admitted sessions. An abusive or incomplete connection can be dropped earlier when it reaches an admission ceiling or deadline.

## Threat model and phases

The relevant attacker can create TCP connections or QUIC packets, stall or fragment input, send malformed handshakes, and, after authenticating as a Pack member, request expensive but authorized DNS resolution and target connection attempts. It cannot defeat TLS, Ed25519, X25519, or AEAD authentication.

| Phase | TCP | QUIC |
| --- | --- | --- |
| P0 | TCP arrival before acceptance | UDP/QUIC arrival handled by Quinn |
| P1 | accepted socket and TLS handshake | Quinn Incoming, then QUIC/TLS establishment |
| P2 | TLS 1.3 complete; challenge/OPEN incomplete | QUIC/TLS complete; first stream/OPEN incomplete |
| P3 | sender signature verified | sender signature and exporter-bound OPEN verified |
| P4 | Pack membership confirmed | Pack membership confirmed |
| P5 | target authorized and DNS resolved if needed | same authorization and DNS rule |
| P6 | target connected and authority lease published | target connected and authority lease published |

Receiver authentication remains before TCP OPEN disclosure. QUIC derives its exporter binding after the completed transport handshake; no OPEN, authorization, or forwarding action uses 0-RTT.

## Baseline audit

Before this stage, application handshake messages were already bounded to 4,096 bytes, the challenge to 512 bytes, and each bounded reader used a monotonic deadline. TCP encrypted data validates its four-byte length against 4,096 before allocating ciphertext. A Pack is validated to at most 1,024 records, so its current structural membership lookup is bounded. Target text is structurally bounded to 512 bytes before DNS.

The pre-existing shared handshake permit was acquired before TCP or QUIC worker creation. TCP TLS already had a five-second read deadline and an eleven-second complete server-handshake deadline. QUIC already owned connection supervisors and per-connection replay/open caps. The audit identified three hardening gaps:

1. Quinn general-purpose defaults allow 65,536 pending Incoming objects and 100 MiB of aggregate Incoming buffering.
2. Authenticated DNS authorization and target connection attempts had a deadline but no dedicated global or per-peer budget.
3. Routine failed handshakes and forwarding events could produce unbounded stderr output; raw TCP pre-auth work waited for its deadline after Silver instead of observing Silver cancellation.

## Exact limits and admission

The following are compiled defaults. Rejected work never waits in an unbounded queue.

| Resource | Limit | Lifetime / release |
| --- | ---: | --- |
| TCP and QUIC sender-handshake permits, shared | 256 global | RAII on error, timeout, cancellation, or completed establishment |
| Sender-handshake permits after verified identity | 16 per Pack peer | RAII; claimed identities are not counted before signature verification |
| QUIC application connection contexts | 128 global | acquired before the connection worker is spawned |
| Quinn pending Incoming | 64 | immediately refused/dropped when full |
| Quinn incoming buffer | 16 KiB each, 1 MiB aggregate | limited before application acceptance |
| QUIC bidirectional streams | 64 per connection | transport parameter; equals Stage11C stream ceiling |
| QUIC unidirectional streams | 0 | Werewolf does not use them |
| QUIC concurrent OPEN processing | 8 per connection | replay-owned semaphore |
| QUIC replay reservations | 256 per connection | connection-owned and released at close |
| DNS plus target-connect work | 64 global, 8 per authenticated peer | one RAII permit covers authorization and connect |
| Target authorization/connect deadline | 5 seconds shared | resolution and connect have one deadline |
| Authority sessions | 1,024 global, 64 per peer, 64 QUIC streams/connection | unchanged Stage11C limits |

TCP takes a shared handshake permit before spawning a JoinSet child, so it does not create a task merely to wait. TLS has a five-second deadline. The post-TLS challenge/OPEN path is bounded by the eleven-second server window and five-second reader window. Silver is checked before admission and watched during TLS and post-TLS work, so a stalled pre-auth TCP worker is cancelled promptly.

QUIC takes both its context permit and shared handshake permit before spawning its connection supervisor. Connection completion and the first bidirectional stream each have a five-second deadline. Each accepted stream takes a handshake permit before its worker is spawned. The QUIC idle timeout remains explicitly finite at 30 seconds; no keepalive is configured.

QUIC Retry was evaluated but is not enabled: the production path uses Incoming::refuse at saturation and never invokes Incoming::retry. Quinn normal anti-amplification remains in force. Source-IP quotas and token buckets were deferred because they penalize NAT, shared gateways, mobile clients, and privacy networks without replacing hard resource bounds.

## QUIC windows and memory model

The transport configuration sets a 64 KiB stream receive window, 512 KiB connection receive window, 512 KiB send window, 16 KiB crypto buffer, and no DATAGRAM receive buffer. The connection window caps aggregate remote stream data, so 64 streams do not multiply the 64 KiB limit into unbounded per-connection allocation. The transport test opens 64 bidirectional streams, verifies no unidirectional credit, and verifies that a 65th bidirectional stream has no credit while the first 64 remain open.

Conservative configuration-level bounds are:

- pending Quinn Incoming data: at most 1 MiB aggregate;
- established QUIC flow-control/send/crypto windows: at most approximately 128 × (512 KiB receive + 512 KiB send + 16 KiB crypto), about 130 MiB of configured capacity, excluding allocation overhead;
- replay reservations: at most 128 × 256 = 32,768 compact entries, with implementation overhead not claimed as a precise byte value;
- application forwarding leases: at most 1,024 across TCP and QUIC.

These are bounds on configured or Werewolf-owned state. Kernel socket buffers, rustls and Quinn internals outside named windows, resolver internals, and allocator overhead are not claimed as exact RSS bounds.

A conservative TCP-heavy FD model has 1,024 authority sessions with an inbound and target socket each, 256 pre-auth TCP sockets, listeners, control socket, and transient target connects. Deployments using the 1,024-session limit need a soft FD limit of at least 4,096. QUIC shares one UDP endpoint but can have up to 1,024 target sockets. This is an operational requirement, not a claim that the common 1,024 FD default is sufficient.

The remotely induced task model is finite: at most 256 shared handshake workers, 128 QUIC connection supervisors, and 1,024 authority-backed forwarding/establishment leases. TCP and QUIC own children through JoinSet/cancellation paths; permits and leases use RAII on timeout, reset, disconnect, revocation, Silver, target failure, and task cancellation.

## Expensive work, fairness, and locking

Parsing and structural bounds happen before signature verification, target resolution, or target connection. Pack lookup is bounded by the 1,024-record validated Pack. DNS runs once per authorization attempt under the shared five-second deadline; the same 64-global/8-per-peer permit covers target TcpStream::connect. A sender only acquires its per-peer target budget after its signature verifies.

The Stage11C authority limits are unchanged. Authority's short synchronous gate and Admission's short synchronous maps only perform local accounting; they are not held across TLS, a network read, DNS, target connect, forwarding, or an await. Daemon-state locks are reduced to clones and bounded local Pack/policy snapshots before network work begins.

There is no source-IP quota and no reserved pre-auth capacity. A distributed flood, or one source that continuously refills the shared 256 handshake slots, can delay a legitimate connection until a slot becomes available. Five-second deadlines and immediate saturation rejection bound recovery after a stopped flood; they do not provide fairness or DoS immunity.

## Saturation, malformed input, and lifecycle results

New loopback tests use test-sized admission limits while exercising production listener code:

- two raw TCP peers send one TLS byte and hold their connections;
- both permits become occupied;
- a third arrival is dropped without a waiting worker;
- after the five-second TLS deadline, permits return and a correct selected-peer TLS connection succeeds;
- a completed TLS connection with no OPEN, and one with a partial OPEN, each release admission at the handshake deadline;
- Silver cancels an in-progress raw TLS handshake without waiting for its deadline.

Admission unit tests saturate and release global/per-peer target work, global handshake work, QUIC contexts, per-connection OPENs, and replay reservations. The malformed frame test sends only a 4,097-byte ciphertext length and proves no ciphertext is read or allocated and the nonce counter remains unchanged. Existing strict parser, replay, invalid signature, unknown peer, malformed target, unauthorized target, and QUIC OPEN tests remain normal workspace tests.

Silver advances the authority epoch and cancels leases. New TCP admissions are dropped while locked and existing TCP pre-auth work observes the Silver watch. QUIC already watches the same source in its connection supervisor. After Pack revocation linearizes, a stale ticket cannot reserve, publish, or submit a frame; cancellation returns resources. Existing authority/revocation tests exercise both transports under active work.

Routine unauthenticated TCP/QUIC handshake failures and per-frame forwarding events no longer emit a line for each remote-triggerable event. Fatal listener and local configuration errors remain operational diagnostics. No metrics subsystem was added. The bounded local control-plane tests continue to pass.

## Regression, dependency, and secret audit

| Gate | Result |
| --- | --- |
| cargo fmt --check | PASS |
| cargo clippy --workspace --all-targets | PASS; pre-existing workspace warnings only |
| cargo test --locked --offline --workspace | PASS: 181 Rust tests |
| Python integration discovery | PASS: 64 tests |
| acceptance runner | PASS: 12/12 |
| git diff --check | PASS |

The sandboxed workspace-test attempt reports eleven expected failures in the
PrivateDirectory ownership/mode tests because its synthetic filesystem metadata
is intentionally rejected as unsafe. This is an environment-only limitation,
not a product failure: the required unsandboxed offline workspace gate passed
all 181 tests against real filesystem metadata.

Rust breakdown is 22 werewolf-core unit tests, 12 core integration tests, and 147 werewolfd tests. No historical test was deleted, ignored, cfg-disabled, or filtered from the normal workspace run. The suite retains Stage9 authorization; Stage10 replay, KDF and AEAD framing; Stage11A fallback; Stage11B persistence; Stage11C revoke/Silver; Stage12 receiver auth and semantic opacity; Stage13B coalescing; Stage14 hygiene; and Stage15B manifest tests. The acceptance runner passed TCP 50 MiB hash integrity and transport fallback/profile checks.

No dependency changed. Cargo.lock remains SHA-256 09b18cac90690855e4bee28cabc59ee713f14baf50f2efbb20f2910ed46047b4.

No secret-bearing artifact was added. The final Stage16 audit found no Pelt or PKCS#8 private material, X25519/shared/session keys, TLS exporter or traffic secrets, key logs, or application payloads in retained source, tests, or this document.

STAGE16_SECRET_MATERIAL_AUDIT = PASS

## Claims and nonclaims

Stage16 certifies finite ceilings for remote application admission, handshake lifetime, QUIC stream/window state, replay state, and authenticated DNS/connect work; saturation fails closed and the tested daemon recovers without restart. It does not claim zero cost in Quinn, the kernel, a resolver, or the network. It does not claim resistance to distributed bandwidth exhaustion, CPU exhaustion from valid cryptographic work below ceilings, FD misconfiguration, or starvation while an attacker continuously occupies every pre-auth slot.

| Claim | Result |
| --- | --- |
| C1: pre-auth TCP work has finite concurrency and lifetime | CERTIFIED |
| C2: stalled TLS/TCP handshakes release bounded resources | CERTIFIED |
| C3: network-controlled lengths do not cause unbounded application allocation | CERTIFIED |
| C4: QUIC unauthenticated connection/stream state has finite explicit limits | CERTIFIED |
| C5: QUIC receive-window multiplication has a documented finite bound | CERTIFIED |
| C6: replay/admission state remains bounded under hostile input | CERTIFIED |
| C7: authenticated DNS/connect work has global and per-peer bounds | CERTIFIED |
| C8: authority remains 1,024 global / 64 peer / 64 QUIC streams | CERTIFIED |
| C9: saturation rejects without an unbounded queue or task creation | CERTIFIED |
| C10: after a stopped flood, permits recover and a legitimate peer connects | CERTIFIED |
| C11: Silver and revocation remain effective under establishment load | CERTIFIED |
| C12: no attacker-controlled operation holds a global lock across blocking network work | CERTIFIED |
| C13: tested externally reachable malformed input paths do not panic | CERTIFIED |
| C14: valid-session wire format did not change | CERTIFIED |
| C15: Stage9 through Stage15B guarantees remain intact | CERTIFIED |
| C16: residual distributed/CPU/network DoS risk is explicitly documented | CERTIFIED |

STAGE16_PREAUTH_RESOURCE_HARDENING = CERTIFIED
