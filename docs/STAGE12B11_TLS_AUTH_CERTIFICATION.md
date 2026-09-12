# Stage 12B.11 — CertificateVerify / negative-authentication certification

`STAGE12B11_CERTIFICATEVERIFY_NEGATIVE_AUTH = COMPLETE` for the isolated,
eight-case TLS 1.3 matrix below. This certifies the experiment, not a production
transport implementation or every possible malformed certificate.

Starting repository: branch `codex/stage12-hide-wire-opacity`, clean HEAD
`3d2a57de968ac74b1155104c1e087b3a6f289315`.

The supplied Stage 12 handoff certifies TCP TLS wire-opacity characterization.
Its earlier authentication-negative certification was withdrawn because D/E
were hardcoded and TEST3 corrupted generic handshake output. This report uses
new executable evidence for those gaps. It does not retroactively validate the
old tests or rerun the earlier passive-capture experiment.

## Harness and authentication profile

[The standalone harness](../tests/stage12_tls_auth/src/main.rs) has its own
manifest and lockfile, outside the production Cargo workspace. It uses rustls
0.23.40, rustls-webpki 0.103.13, rcgen 0.13.2, and ring 0.17.14. Production
sources and the root manifests/lockfile are unchanged. No Quinn code is built
by this harness and no Quinn endpoint is started.

Two rustls connection state machines exchange serialized TLS records through
in-memory byte buffers. These are real TLS handshakes and encrypted records;
there is no TCP socket or packet capture in this experiment. The transport
pump never edits handshake bytes or ciphertext.

The client enables TLS 1.3 only, uses IP `ServerName` `127.0.0.1`, offers no
ALPN, disables resumption and early data, and has no client certificate. The
server sends no session tickets. Every case asserts TLS 1.3, `SNI=None`,
`ALPN=None`, and zero TLS 1.2 verification callbacks.

The experimental verifier parses the actual peer certificate with
`webpki::EndEntityCert::try_from`, extracts its SPKI, requires canonical Ed25519
SPKI encoding (absent parameters, zero unused bits, 32-byte key), and compares
the complete SPKI against the expected receiver key. It then delegates TLS
1.3 CertificateVerify checking to rustls's real `verify_tls13_signature`.
The verifier has no case name, expected-outcome flag, or negative-case switch.
Expected outcomes are assertions made after the connection runs.

Fresh Ed25519 keys A and B represent Pelt-format identities. These are temporary
test keys, not identities loaded through the production Pack/Pelt APIs. No
private keys are persisted. The expected pin is selected before the handshake;
there is no CA, hostname, certificate self-signature, or TOFU trust decision.
The certificate DNS names intentionally differ from the IP ServerName.

## Observed matrix

| Case | Actual peer input | Certificate decision | CertificateVerify | Result |
| --- | --- | --- | --- | --- |
| A | Certificate A, signer A, pin A | Exact SPKI accepted | Verified | Both handshakes complete; exact 100 bytes recovered |
| A2 | Different certificate/DNS name, same key A | Exact SPKI accepted | Verified | Both handshakes complete; exact 100 bytes recovered |
| B | Certificate B, signer B, pin A | Pin mismatch | Not called | `ApplicationVerificationFailure`; peer `AccessDenied` |
| C | Unchanged certificate A, signer B, pin A | Exact SPKI accepted | `BadSignature` | Peer `DecryptError` |
| C2 | Unchanged certificate A, signer A, one signature bit flipped | Exact SPKI accepted | `BadSignature` | Peer `DecryptError` |
| D | Certificate A DER with its final byte removed | Actual DER parser rejects | Not called | `BadEncoding`; peer `BadCertificate` |
| E | Valid, generated ECDSA P-256 certificate | Non-Ed25519 SPKI rejected | Not called | `BadEncoding`; peer `BadCertificate` |
| A3 | Fresh connections using certificate A after all negative cases | Exact SPKI accepted | Verified | Both handshakes complete; exact 100 bytes recovered |

Every negative case asserts exactly one certificate-verification callback,
both handshakes incomplete, a fatal alert received by the server, zero
application bytes submitted, and zero application plaintext recovered.

D traverses the actual DER parser; the observed parser diagnostic is
`TrailingData(SignedData)`. E first passes a fixture sanity check through that
parser and is then delivered through TLS to the same certificate verifier.
It is rejected by the SPKI type/encoding check before pin comparison or
CertificateVerify. E's hostile server resolver intentionally pairs the ECDSA
certificate with the Ed25519 A signer, so the certificate reaches the callback
under the client's Ed25519-only signature offer. This establishes certificate
type rejection; it is not an ECDSA signature-negotiation test.

The hostile resolver also permits mismatched keys and malformed DER in C/D,
which a normal server configuration builder may reject locally. Local builder
rejection would not establish peer-side authentication rejection.

## Why C/C2 establish CertificateVerify failure

Fault injection occurs inside rustls's server `Signer::sign` callback. Before
signing, the harness asserts the exact TLS 1.3 server CertificateVerify context:
64 space bytes, `TLS 1.3, server CertificateVerify`, the terminating NUL, and a
32- or 48-byte transcript hash. All emitted signatures have 64 bytes.

C signs that message with B while sending the unchanged A certificate. C2
obtains A's valid signature and flips precisely one bit in its first byte,
before rustls serializes and encrypts CertificateVerify. There is no TLS record,
AEAD tag, Certificate message, or generic handshake-output corruption.

For both cases, executable assertions establish:

- The certificate/SPKI pin was accepted before signature verification.
- The client's verifier receives exactly the message passed to the server
  signer and exactly the signature that signer emitted.
- A separate ring verification call accepts the original signature under the
  actual signing key. For C it rejects that original signature under pin A;
  for C2 it accepts the original under A and rejects the altered signature.
- The real rustls TLS 1.3 verifier returns
  `InvalidCertificate(BadSignature)`, which becomes the connection's error.
- The peer receives `AlertReceived(DecryptError)`.

The direct ring cross-check uses the same cryptographic backend as rustls; it
is a separate verification invocation, not an independent implementation.

## Application-data boundary

The harness calls the application writer only after both connections finish
successfully and recorded pin/signature checks pass. Positive cases recover
the exact 100-byte harness sentinel (`0x57` repeated 100 times). No application
writer call occurs in any negative case. This validates the experiment's
authentication gate. Production OPEN encoding, integration, buffering, and
reconnection behavior are outside this stage's evidence.

## Reproduce and inspect evidence

From the repository root:

```sh
cargo run --locked --offline --manifest-path tests/stage12_tls_auth/Cargo.toml --target-dir target/stage12-tls-auth
cargo run --release --locked --offline --manifest-path tests/stage12_tls_auth/Cargo.toml --target-dir target/stage12-tls-auth
cargo fmt --manifest-path tests/stage12_tls_auth/Cargo.toml -- --check
cargo clippy --locked --offline --manifest-path tests/stage12_tls_auth/Cargo.toml --target-dir target/stage12-tls-auth -- -D warnings
```

This is an assertion-driven executable: use `cargo run`, not `cargo test` to
exercise the matrix. A failed assertion exits unsuccessfully; JSON is emitted
only after all cases pass. Regular assertions also run in the release build.
The standalone lockfile requires the listed dependencies to be cached for
offline execution.

[Recorded evidence](../tests/stage12_tls_auth/evidence.json) contains debug and
release runs, tool versions, source/manifest/lockfile SHA-256 hashes, and the
starting HEAD. Case records include certificate hashes, public pins, real
callback counts, cryptographic outcomes, errors, alerts, handshake states,
and byte counts. Fresh keys make certificate hashes and some record lengths
vary between runs; the asserted security outcomes must remain the same.

Validation: debug and release matrices pass (8/8 each), formatting passes, and
Clippy passes with warnings denied. No production regression suite was run:
the change adds only this isolated harness and documentation.
