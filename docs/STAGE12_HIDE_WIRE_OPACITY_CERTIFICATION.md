# Stage 12 hide / wire-opacity certification

**STAGE12_HIDE_WIRE_OPACITY = CERTIFIED** for the source tree at
`27fe8e9da815832919e3b16279b720f42c79ad17` on
`codex/stage12-hide-wire-opacity`. All twelve required statements below are
certified. This document is the documentation-only certification commit; the
certified source HEAD is recorded above because a commit cannot contain its
own hash. The S10 lost-wake race was fixed in `35f15cb`, and the Stage 12E.2
full-production TCP capture in `27fe8e9` closes S1.

## Scope and threat model

The Stage 12 contract is authenticated selected-peer transport before
application disclosure, with Stage 9 target authorization, Stage 10 replay and
channel binding, and Stage 11C authority remaining independent. It addresses
plaintext semantic disclosure to passive wire observers and premature
application disclosure to fake receivers. It does not promise anonymity or
traffic-analysis resistance. The Stage 11 security review had established
separate sender authentication, target authorization, replay, authority and
fallback controls, but those controls alone did not authenticate a QUIC
receiver's Pelt or hide the raw TCP Stage 10 challenge/OPEN/ACK. Stage 12
closes those receiver-trust and plaintext-wire gaps while keeping the Stage 9,
10 and 11 controls independently enforced.

## Before/after architecture and common Pelt TLS trust root

Before Stage 12, production QUIC used a generated `localhost` certificate and
a skip verifier; encrypted TCP sent its Stage 10 inner protocol over raw TCP.
After Stage 12, both transports use a process-lifetime certificate derived from
the existing daemon Pelt Ed25519 key. The selected Pack peer's **full** public
key becomes canonical SPKI through ed25519-dalek. rustls-webpki parses the
leaf certificate, and the verifier compares its full SPKI exactly with that one
selected key. rustls performs genuine TLS 1.3 CertificateVerify. There are no
CA roots, hostname trust, TOFU, or fingerprint-only fallback.

The daemon derives `RuntimeTlsIdentity` once from its existing Pelt SigningKey
via in-memory PKCS#8 and rcgen, then verifies the resulting certificate SPKI
against the Pelt public key. No independent or persistent TLS identity is
created. The fingerprint remains display/index material, not TLS trust
material. Each connection pins exactly its selected Pack peer.

## QUIC and encrypted-TCP constructions

For QUIC, `Connecting.await` precedes the unchanged Stage 10 exporter, stream
creation and encrypted OPEN. For encrypted TCP, TLS authentication precedes the
existing challenge/OPEN, inner X25519, BLAKE3 KDF, ChaCha20-Poly1305 framing,
ACK and forwarding. Source and fixed-vector tests preserve the Stage 10
transcripts, exporter label `EXPORTER-WerewolfProxy-Fang-QUIC-v3` and context
`werewolfproxy/fang-quic-v3`. These strings are local inputs or encrypted
application content, not plaintext transport labels.

Both shared rustls factories explicitly allow TLS 1.3 only, disable client
resumption and early data, set server session storage to no storage, set TLS 1.3
tickets and server early data to zero, leave ALPN empty and require no client
certificate. Production TCP uses IP `ServerName`; Quinn receives the textual IP
which rustls interprets as IP `ServerName`. Production-path tests observe no
DNS SNI and no ALPN. The selected full Pack key is validated before outbound
TCP or UDP connection setup. QUIC never calls `into_0rtt`.

## Pelt lifecycle and lost-wake correction

At `c1eba70`, `control/mutation.rs` constructed a candidate certificate,
validated its SPKI, persisted Pelt through the existing durability coordinator,
then published Pelt and `RuntimeTlsIdentity` under one state lock. It called
`tls_identity_ready.notify_waiters()` after releasing that lock. Pre-persistence
failure publishes neither, an indeterminate persistence result does not
publish an operational identity, repeated `pelt.init` does not rotate Pelt,
and `pelt.init` does not unlock Silver. Existing-Pelt startup constructs one
runtime identity from the loaded key.

In that pre-fix revision, both `transport/tcp_encrypted.rs:32-40` and
`transport/quic.rs:31-43` checked for identity under the state lock, cloned
the `Notify`, released the lock, and **then** constructed and awaited
`ready.notified()`. A successful `pelt.init` could call `notify_waiters()` in
that gap. Tokio does not retain a
`notify_waiters()` event for a waiter created afterward. A disposable proof
using the locked Tokio 1.52.3 reproduced that exact ordering and observed
`NOTIFY_WAITERS_BEFORE_FIRST_POLL_LOST=true`. The listener could consequently
remain asleep although state contained a valid Pelt and TLS identity. This
was an availability/lifecycle failure, not evidence of identity mismatch or
an authentication bypass. Pre-fix same-process tests and acceptance exercised
a favorable schedule; they did not exclude this interleaving.

Commit `35f15cb` replaces the one-shot `Notify` with a persistent Tokio
`watch<bool>`. Listener code subscribes before checking runtime state;
`pelt.init` advances watch only after durable persistence and coherent Pelt/TLS
publication. Existing-Pelt startup initializes watch as ready. A deterministic
test pauses the production wait helper after it observes absent identity,
publishes the identity, then lets it install its watch wait; it completes within
a one-second timeout. A live test starts both actual TCP and QUIC listeners,
observes each waiting, publishes once, and authenticates both in-process.
Multiple early/late subscribers, failed persistence, indeterminate outcome,
repeated initialization, and Silver locking have regression coverage. The
workspace passed with 145 Rust tests immediately after the fix. The final
146-test workspace also passed under umasks 0000, 0002 and 0077; Python
passed 64 tests and acceptance passed 12/12. Both S10 and S1 are closed.

## QUIC passive-observer and fake-receiver evidence

The production QUIC receiver-auth tests pass for the correct selected peer and
50 MiB transfer. Wrong Pelt and another valid Pack peer fail before application
stream creation. Missing or malformed full keys fail before UDP output. The
fake receiver receives zero OPEN, target, sender application metadata, and
application bytes. The production client test observes no SNI or ALPN and
matches exporter-derived binding across endpoints. The standalone locked
Quinn proof also passed correct/wrong/other-peer cases, no ALPN, disabled
0-RTT/resumption and public QUIC Initial inspection: no SNI, ALPN or
application OPEN/target/sender semantics. Standard Initial protection has
publicly derivable keys; QUIC v1 version, destination connection ID, packet
number, TLS cipher suites, supported version and ClientHello extension types
remain observable. The decrypted proof ClientHello has SNI null, ALPN empty,
no early-data or pre-shared-key extension, and extension types 51, 13, 45, 23,
57, 5, 43, 10 and 11. The raw-datagram scans for the correct, wrong and
other-peer cases returned NOT_FOUND for project semantic strings, actual
fingerprints, target, OPEN marker, application sentinel, Pelt raw keys,
certificate DER and SPKI DER. This byte scan is a weaker observation than
Initial decryption and is not presented as proof of Initial secrecy.

## TCP fake-receiver and full-production passive-wire evidence

The production encrypted-TCP tests pass for the correct selected peer, reject
wrong and other Pack Pelts before the inner protocol, and reject missing and
malformed Pack keys before connection. A production-forwarder fake receiver
receives zero application bytes. Raw unauthenticated clients receive no
plaintext Stage 10 challenge/OPEN/ACK.

Stage 12E.2 adds a **full-production** successful encrypted-TCP session:
`pipe_one_fang_connection` -> transparent raw byte relay -> `run_fang_listener`
and `server_handshake` -> real TCP echo target. The relay forwarded unchanged
bytes in both directions; it neither terminated nor parsed TLS. A `#[cfg(test)]`
observer inside the production `handshake::read/write` functions retained only
the actual serialized public challenge, OPEN and ACK in test memory. For each
message, exactly one successful production write equaled exactly one
successful production read. The challenge correlated with both OPEN and ACK;
OPEN and ACK contained the actual selected sender/receiver fingerprints,
nonce and target. The production client calls `connect_authenticated_tcp`
before reading the challenge or constructing/submitting OPEN, so
`OPEN_BEFORE_TLS_AUTH=NO`. The server source checks Pack membership, challenge,
signature, replay and authority before Stage 9 target authorization and target
connect; it generates ACK only after target connection/publication. The target
received and echoed the exact 128-byte random sentinel through the unchanged
X25519/ChaCha20-Poly1305 forwarding path. The direct call path selected only
encrypted TCP, with no selector or plaintext fallback.

The sampled run used sender `wwp1:34-3B-73-CD-3B-28-22-8D`, receiver
`wwp1:E7-B4-7E-37-BF-44-28-29` and target `127.0.0.1:45665`. The 128-byte
sentinel's BLAKE3 digest was
`3c78f4ddd1abf067273bf1864ad8be221e9f816cf765a9a87d99055f5d372619`.
Serialized challenge/OPEN/ACK lengths were 187/529/634 bytes; their SHA-256
digests were respectively
`a7979ec6cbc29f1c1a75c09fcb48be52909dc3b056e4bcf2d94f60ab75b26a42`,
`687259211a41d38f565622e18f9d804c499516690cf5c990193ec487817e2e1c`,
and `213967ddbebfb231ca6215c69f20e5c91d38ad21cc2577fc7001eb7a7f42bd23`.
These values identify the in-memory production messages without storing their
complete serialization.

The relay captured 2,321 client-to-server bytes and 3,494 server-to-client
bytes. SHA-256 of the combined capture, defined as client-to-server bytes
followed by server-to-client bytes, was
`4c14f84ade05d813a7ae580ea3e5c355a28249ccea71a2b96e58a3c6e65fb0e8`.
Both directions and their combination returned **NOT_FOUND** for the actual
sender and receiver fingerprints, exact target, exact sentinel, full serialized
challenge/OPEN/ACK, raw sender and receiver Pelt public keys, canonical
sender/receiver SPKI DER and receiver certificate DER. Case-insensitive scans
also returned NOT_FOUND for `werewolf`, `fang`, `pelt`, `pack`, `wwp1`, `sender`,
`receiver`, `target`, `protocol`, `challenge`, `ack`, `fang.tcp.challenge`,
`fang.pipe`, `fang.tcp.ack`, `fang-tcp-v3` and `fang-v3-secure` in each
direction and combined. The raw capture was held only in test memory and was
not committed or persisted. In-memory scans found neither Pelt private seed,
private PKCS#8 nor keylog markers; no TLS traffic secret, X25519 private key
or exporter was instrumented or retained. `CAPTURE_SECRET_AUDIT=PASS`.

This sample and the repeatable test establish S1 for the certified IP/TLS 1.3
path. They do not hide the connection's IP/port, timing, sizes or traffic
volume. An earlier standalone 100-byte TLS sentinel proof remains supporting
evidence, rather than a substitute for this production OPEN/ACK capture.

## CertificateVerify proof and production trust audit

The isolated locked Stage 12B.11 authentication matrix was re-executed: 8/8
pass. Correct Ed25519 CertificateVerify succeeds; wrong-key and bit-flipped
signatures are rejected as `BadSignature`; malformed and non-Ed25519
certificates are rejected by parsing/exact-SPKI validation. Exact SPKI alone
therefore does not bypass the rustls possession proof.

A production source search found `SkipServerVerification` only in the
`#[cfg(test)]` QUIC lab and a standalone diagnostic echo-client binary.
Production outbound QUIC and encrypted TCP call the shared selected-peer
client factory; the verifier's certificate assertion follows exact parsed
SPKI equality, and its TLS 1.3 signature method calls
`rustls::crypto::verify_tls13_signature`. There is no active-production
unconditional signature assertion or insecure receiver-auth fallback.

## Active-participant boundary

An active TLS/QUIC participant can obtain the legitimate server certificate
and derive its Pelt public identity. This accepted boundary is separate from
passive wire opacity and fake-receiver application nondisclosure.

## Stage 9 authorization, Stage 10 replay/crypto, and Stage 11 regressions

The full workspace tests and 12/12 integration acceptance cover Stage 9
sender authentication, resolve/authorization before target connection and
exact canonical IP+port policy. Stage 10 tests retain TCP one-shot challenge,
sender nonce, connection binding, QUIC exporter binding, duplicate OPEN
rejection, reservation capacity/cleanup, and restart separation. Stage 11C
tests retain lease-gated submissions, peer revoke, mixed-peer QUIC streams,
Silver global inbound QUIC closure, prompt TCP cancellation, and graceful
ordinary post-ACK forwarding failure. Stage 11B control, Den no-follow,
durability and mutation tests pass under all requested umasks.

Stage 11A strict mode remains QUIC secure -> encrypted TCP secure -> failure;
plain fallback is limited to explicit compatibility/legacy policy. A failed
receiver authentication does not silently promote plain transport in strict
mode. New secure QUIC rejects historical insecure-cert QUIC, and new encrypted
TCP rejects historical raw inner-v3 TCP; no opportunistic probe was added.

## Large-transfer integrity

The 50 MiB production QUIC test sends and receives exactly 52,428,800 bytes
and compares incremental BLAKE3 hashes. The integration acceptance downloads
52,428,800 bytes over encrypted TCP and compares exact SHA-256 with the source
fixture. Both passed.

## Resource limits, failure oracles, and certificate metadata

Source retains 5-second read, 11-second server and 15-second client handshake
windows; a 4,096-byte message cap and 512-byte challenge cap; 256 global
pending handshakes; authority caps of 1,024 global, 64 per peer and 64 per
QUIC connection; a 2,048-byte TCP frame cap; replay bounds; and owned
`JoinSet` listener tasks. These limit resources but cannot eliminate pre-auth
CPU, bandwidth or connection-exhaustion attacks. Remote failures remain
generic transport/protocol failure rather than detailed plaintext
authorization or crypto errors. Connection success/failure, handshake timing,
closes, resets, record sizes and chosen transport can still provide
distinguishable failure signals; no timing-indistinguishability claim is made.

An actual production certificate captured during this review is a 195-byte
Ed25519 X.509 v3 certificate with empty subject and issuer, no SAN, a
library-generated serial, and validity from 1975-01-01 to 4096-01-01 UTC.
The certificate DER contains none of the project-semantic terms listed in the
contract. Its public certificate/SPKI is not a secret. `RuntimeTlsIdentity`
has no `Debug` implementation and is not persisted or logged.

## Dependencies, history, gates, and secret audit

The current `Cargo.lock` SHA-256 is
`c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17`.
Resolved relevant versions are tokio-rustls 0.26.5, rustls 0.23.40,
rustls-webpki 0.103.13, quinn 0.11.9, quinn-proto 0.11.15, quinn-udp
0.5.14, rcgen 0.13.2, ed25519-dalek 2.2.0, ring 0.17.14 and aws-lc-rs
1.17.0. The existing feature graph compiles ring and aws-lc-rs support, but
the shared TLS factory explicitly selects the ring provider. The rustls
`tls12` crate feature is present through the locked graph; generated configs
explicitly allow only TLS 1.3. There is one resolved rustls/webpki version
each and no added X.509 parser beyond the approved direct rustls-webpki.

The Stage 12 commits after `3d2a57d` are, in order:

1. `b3fced8` Ed25519 PKCS#8 feature; `11ede42` locked webpki dependency;
   `311c1ab` shared identity and verifier.
2. `f25f3c9`, `b979e95`, `a939046` QUIC migration, tests and evidence.
3. `99b7601`, `34aadfd`, `968d6ed`, `2f601e4`, `d599d7f` Tokio rustls adapter,
   TCP migration, tests and evidence.
4. `9b3b829` committed Stage 12B/C proof harnesses/evidence;
   `cb53304`, `ad3c5ee`, `c1eba70` lifecycle change, test and document.
5. `35f15cb` race-free listener activation; `27fe8e9` production TCP
   OPEN/ACK passive-wire regression and test-only observation seam.

`cargo fmt --check`, `cargo clippy --workspace --all-targets`,
`cargo test --locked --offline --workspace`, Python unittest discovery,
`python3 -B tests/integration/run.py`, and `git diff --check` passed at the
final source HEAD. Rust: 146 passed: werewolf-core 21 unit and 12 integration,
werewolfd 113; zero ignored, failed or filtered in normal workspace runs.
The same full workspace passed under umasks 0000, 0002 and 0077. Python:
64 passed. Integration acceptance: 12/12 passed. No Rust test was deleted,
renamed to escape discovery, ignored or disabled by configuration. The Python
receiver-binding test was updated for explicit Stage 12 compatibility
behavior, with its discovery count retained.

Stage 12 source, documentation and committed B/C JSON evidence were audited
for private-seed, private-key/PKCS#8, traffic/session/exporter secret, keylog
and hardcoded production-secret disclosure. Evidence contains public
identities, fingerprints, sentinels, datagram/certificate material and result
flags; the exporter is recorded by equality only. Harnesses generate private
test keys in memory. No secret material was found.
Two literal traffic-secret marker names in the TCP capture test are negative
search terms used to reject keylog leakage, not retained secret values.
`STAGE12_SECRET_MATERIAL_AUDIT = PASS`.

## Required statements

| Claim | Verdict | Basis / limitation |
| --- | --- | --- |
| S1 passive TCP semantics/fingerprints/target opaque | CERTIFIED | Full-production challenge/OPEN/ACK and sentinel path executed through a transparent bidirectional relay; all actual values were NOT_FOUND on the raw wire. |
| S2 passive QUIC OPEN/target/sender opaque | CERTIFIED | Production-path tests and standard Initial inspection; Initial metadata is not confidential. |
| S3 fake TCP receiver gets no OPEN/application semantics | CERTIFIED | Wrong-peer and fake-forwarder production tests, zero application bytes. |
| S4 fake QUIC receiver gets no OPEN/application semantics | CERTIFIED | Wrong/other-peer production tests, stream count zero. |
| S5 trust bound to exactly selected full Pack key | CERTIFIED | Canonical full-SPKI equality and missing/invalid/other-peer rejection. |
| S6 genuine TLS 1.3 CertificateVerify | CERTIFIED | rustls helper and executed correct/wrong/bit-flip signature matrix. |
| S7 missing/invalid full key fails closed | CERTIFIED | Production TCP/QUIC preconnection tests. |
| S8 TLS does not replace sender/replay/authz/authority | CERTIFIED | Unchanged ordering and Stage 9/10/11 regression tests. |
| S9 strict mode cannot downgrade to plain | CERTIFIED | Selector policy and 12/12 acceptance fallback matrix. |
| S10 fresh `pelt.init` reliably activates secure listeners in-process | CERTIFIED | `35f15cb` uses persistent watch readiness; deterministic interleaving and live dual-listener tests pass. |
| S11 Silver/revoke authority semantics remain enforceable | CERTIFIED | Stage 11C tests and unchanged authority gate. |
| S12 Stage 10 TCP inner crypto and QUIC exporter unchanged | CERTIFIED | Source/fixed vectors, exporter equality and replay suite. |

All S1–S12 claims are certified against the source HEAD stated at the top of
this document. The final review also re-executed the standalone TLS
authentication matrix (8/8) and Quinn receiver-auth proof; full gates and the
three-umask workspace matrix passed. The certification commit changes this
document only and does not modify the certified production source.

Out of scope or remaining limitations: source/destination IP and ports,
TCP versus QUIC, timing, packet and record sizes, traffic volume, connection
duration and traffic patterns remain observable; there is no TLS/QUIC
fingerprint camouflage, global traffic-correlation resistance,
active-participant receiver anonymity, post-quantum security or
rollback-protected Den storage. Failure timing/reset behavior and pre-auth
resource exhaustion remain possible. Active clients can inspect the public
receiver certificate.
