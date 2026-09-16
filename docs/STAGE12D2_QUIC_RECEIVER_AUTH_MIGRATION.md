# Stage 12D.2 — Production QUIC receiver authentication migration

`STAGE12D2_QUIC_RECEIVER_AUTH_MIGRATION = COMPLETE`.

Production QUIC now presents the daemon's one process-lifetime Pelt-backed TLS
identity and authenticates the one selected Pack peer through exact canonical
SPKI equality plus genuine TLS 1.3 CertificateVerify. The existing Stage 10
exporter, OPEN/ACK transcripts, sender authentication, replay, Stage 9 target
authorization, Stage 11C authority, and selector behavior are unchanged. TCP
transport code is unchanged.

## Pre-change audit

Before this migration:

- `transport/quic.rs::run_quic_fang_listener` called
  `quic_lab::make_server_endpoint`, which generated a new rcgen `localhost`
  certificate unrelated to the daemon Pelt.
- `quic_fang.rs::connect_v3` called `quic_lab::make_client_endpoint`, whose
  `SkipServerVerification` accepted the certificate and both TLS signature
  callbacks by assertion.
- The Quinn connection name was the DNS string `localhost`; ALPN was left at
  its empty default.
- Production did not call `into_0rtt`, but its generic client/server configs did
  not explicitly carry the Stage 12 disabled-resumption policy.
- `Connecting.await` already preceded exporter derivation, `open_bi`, and the
  Stage 10 OPEN.
- `handshake.rs::quic_binding` used the exact label
  `EXPORTER-WerewolfProxy-Fang-QUIC-v3` and context
  `werewolfproxy/fang-quic-v3`.

## Production result

`main.rs::open_fang_with_profile` now copies the selected `PeerRecord`'s
optional full public key into `ExpectedPeerIdentity`. `quic_fang::connect_v3`
passes that key to the shared Stage 12 client factory before creating a Quinn
endpoint. Missing or noncanonical key material therefore fails before the
first UDP datagram; there is no fingerprint, CA, hostname, TOFU, any-Pack-peer,
or skip-verification fallback.

The shared rustls config is converted with
`quinn::crypto::rustls::QuicClientConfig::try_from`. It retains TLS 1.3 only,
the exact selected-peer verifier, genuine CertificateVerify, disabled
resumption and early data, and empty ALPN. The connection name is the textual
remote IP. The locked Quinn/rustls stack parses it as `ServerName::IpAddress`;
the live server-side handshake observation records `server_name=None` and
`protocol=None`.

`transport/quic.rs::run_quic_fang_listener` clones the process-lifetime
`RuntimeTlsIdentity` from daemon state, obtains the shared hardened server
config, converts it through `QuicServerConfig::try_from`, and reuses the result
for the listener's connections. No QUIC-specific identity is generated,
persisted, or logged. A missing runtime identity fails listener construction.

Normal `Connecting.await` remains mandatory. Only after it succeeds does the
client derive the unchanged Stage 10 exporter binding, call `open_bi`, and
submit the existing OPEN. The server still authenticates each sender at the
application layer and preserves all replay, admission, authority, target, ACK,
and forwarding ordering. It uses no TLS client certificate.

The production daemon no longer compiles or calls `quic_lab`; that module is
test-only in the daemon. Standalone lab/debug binaries retain their explicitly
isolated skip-verification helpers and are not selected by production transport
code.

## Executed receiver-authentication matrix

Seven new Rust tests exercise real UDP loopback and production connection
logic:

| Case | Executed result |
| --- | --- |
| Selected A, server Pelt A | PASS through the production listener and client |
| 50 MiB application path | PASS; exact incremental BLAKE3 equality at the TCP target |
| Client expects A, server Pelt B | TLS/QUIC reject; application stream count 0 |
| Client expects A, server Pelt C, with A/C valid Pack records | reject; application stream count 0 |
| Selected record has no full key | reject before UDP; datagram count 0 |
| Selected record has invalid/noncanonical key | reject before UDP; datagram count 0 |
| IP ServerName / empty ALPN | server observes SNI `None` and ALPN `None` |
| Exporter | client/server bindings match with the unchanged label/context |
| OPEN/ACK | commands, protocol, fields, signatures, and binding remain valid |
| Ordinary target close immediately after ACK | client still receives and authenticates ACK |

Wrong/fake receivers accept no application stream, so their observed OPEN,
target, sender-identity, and application byte counts are all zero. Another
valid Pack peer is not accepted as the selected peer.

The successful instrumentation also obtains the peer certificate through
`Connection::peer_identity` and matches it to the server certificate. This
confirms the accepted Stage 12 boundary: an active QUIC participant can learn
the receiver's certificate/Pelt identity.

The factories' existing executable tests establish TLS 1.3 only, client
resumption disabled, server session storage/tickets disabled, client/server
early data disabled, and empty ALPN. Production calls no early-connection or
`into_0rtt` API, and wrong-receiver tests observe zero streams.

## Wire observation

A UDP relay captures the actual production client's datagrams while forwarding
them unchanged to a server using the production shared server config. The
server recovers the exact 100-byte random application sentinel after the
authenticated OPEN/ACK exchange. The recorded runtime scan result was:

```text
werewolf=NOT_FOUND  fang=NOT_FOUND      pelt=NOT_FOUND
pack=NOT_FOUND      wwp1=NOT_FOUND      target=NOT_FOUND
sender=NOT_FOUND    receiver=NOT_FOUND  ack=NOT_FOUND
protocol=NOT_FOUND
```

Exact sender/receiver fingerprints, target, both public-key encodings, receiver
raw32, receiver certificate DER, receiver SPKI DER, and the application
sentinel were also `NOT_FOUND` in raw datagram payload bytes.

This raw scan is not a claim that QUIC Initial packets are opaque. The server's
decrypted ClientHello observation proves that this production client sends no
SNI or ALPN. The separately preserved Stage 12C public Initial decoder shows
that the locked Quinn/rustls stack exposes ordinary ClientHello cipher suites,
extension ordering, key shares, and QUIC transport parameters after standard
Initial decryption. This stage makes no traffic-analysis, fingerprint
camouflage, address, port, timing, or size-hiding claim.

## Regression and compatibility results

The final worktree passed:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets` (existing warnings only)
- `cargo test --locked --offline --workspace`: 132 passed, 0 failed, 0 ignored
- `python3 -B -m unittest discover -s tests/integration -p 'test_*.py'`:
  64 passed
- `python3 -B tests/integration/run.py`: 12 acceptance scenarios passed
- `git diff --check`

The Rust aggregate increased from 125 to 132 through seven new tests. Breakdown:
`werewolf-core` library 21, `werewolf-core` integration tests 12, and
`werewolfd` 99. Other binary/doc-test targets contain zero tests. No Rust test
was deleted, ignored, or renamed.

Focused regressions also passed: the two Stage 9 live authorization matrices;
four Stage 10 QUIC replay/capacity tests plus the exporter equality/separation
test; ten Stage 11A selector tests; the Stage 11C multiplexed peer
revoke/Silver test; and the live post-ACK target-close test. Full acceptance
passed QUIC application traffic, strict/compatibility/legacy fallback matrices,
profile restoration, daemon restart, and raw TCP targets. The 50 MiB QUIC path
is covered by the new production Rust test; the acceptance runner's existing
50 MiB encrypted-TCP SHA-256 scenario also remains green.

The historical interoperability matrix passes with the intended Stage 12
result: an old insecure QUIC server certificate is rejected rather than causing
restoration of `SkipServerVerification` or a downgrade. Selector order remains
QUIC, encrypted TCP, then fail for strict policy.

## Dependency and evidence audit

No manifest or lockfile changed. `rustls-webpki` remains 0.103.13 and the
provider graph remains the previously certified unified `ring`/`aws-lc-rs`
state. `Cargo.lock` SHA-256 is
`4d7d44d53862bcd4e3ffe727be11920c06a414651401f67c69b90ebad98fc3c5`.

The preserved Stage 12B/C documents, manifests, harness sources, lockfiles, and
JSON were audited. JSON contains public identity/certificate material, hashes,
test sentinels, raw encrypted datagrams, callback/result metadata, and exporter
equality only. It contains no Ed25519 private seed, private PKCS#8/TLS key, TLS
traffic secret, exporter bytes, or key-log material. Harness source handles
fresh ephemeral private material in memory but neither embeds nor emits it.
The raw proof artifacts remain untracked and were not added to these commits.

Implementation commit: `f25f3c9` (`feat(quic): authenticate receivers with
Pelt SPKI`). Test commit: `b979e95` (`test(quic): cover Stage 12 receiver
authentication`).
