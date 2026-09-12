# Stage 12D.3 — Production encrypted-TCP receiver authentication

`STAGE12D3_TCP_RECEIVER_AUTH_MIGRATION = COMPLETE`.

Encrypted TCP now authenticates the selected Pack receiver with TLS 1.3,
exact canonical Pelt SPKI equality, and genuine rustls CertificateVerify
before reading the local application stream or sending the existing Stage 10
OPEN. The existing TCP challenge/OPEN/ACK transcript, X25519 key derivation,
padded ChaCha20Poly1305 framing, sender Pack verification, replay, authority,
target authorization, selector, and production QUIC are unchanged.

## Previous TCP behavior and integration

Before migration, `transport/tcp_encrypted.rs::run_fang_listener` accepted a raw
TCP stream and sent the Stage 10 challenge in cleartext. Its client consumed
the challenge and sent OPEN before authenticating the selected receiver. The
ACK was signed but the selected peer's full Pack public key was not used for
transport authentication. There was no TLS ServerName, ALPN, resumption, or
0-RTT configuration for this transport.

The listener now reuses the process-lifetime `RuntimeTlsIdentity` derived
from its current daemon Pelt. It refuses to start without that identity. The
client builds the shared exact selected-peer verifier from
`ExpectedPeerIdentity.public_key_b64` before opening a TCP socket. Missing or
noncanonical full keys fail closed with no TCP connection and no fallback to
fingerprint, CA, hostname, TOFU, or skip verification. The client uses the
connected peer's IP as `ServerName::IpAddress`; the live TLS server observes
SNI `None` and ALPN `None`. Both sides use the shared TLS-1.3-only configs
with disabled resumption/tickets and early data and empty ALPN.

Only after the normal TLS handshake succeeds does the client receive the
encrypted Stage 10 challenge, send OPEN, verify ACK, and forward application
bytes. The server still validates the sender's Pack membership and signature,
replay and authority, target policy, and ACK independently of TLS. It does
not use a client certificate.

The Stage 11C authority submission gate now wraps the **raw TCP stream below
rustls**. Before sender verification, TLS handshake/challenge writes are
allowed. Installing the sender lease is a one-way transition immediately
after sender authentication; every subsequent raw write, flush, and shutdown
poll uses the existing authority gate. This prevents buffered TLS ciphertext
from escaping after revocation, including ACK and forwarded traffic. The
target socket retains its existing raw `AuthorityWriter` gate. Existing TCP
revoke/Silver tests pass with the TLS stream.

## Executed authentication and wire results

Six new Rust tests use actual loopback sockets:

| Case | Result |
| --- | --- |
| Selected Pelt A; server Pelt A | TLS 1.3 succeeds; server recovers exact random 100-byte sentinel only after client authentication |
| Wrong Pelt B; another valid Pelt C | Client rejects both during TLS; fake receiver application byte count 0 |
| Production forwarder against wrong receiver | Rejects; fake receiver receives 0 OPEN, 0 target, 0 sender identity application bytes |
| Missing/invalid selected Pack full key | Rejects before any TCP connection |
| ServerName and ALPN | Server observes SNI `None` and ALPN `None`; client receives the matching runtime certificate |
| Raw TLS relay | Recovered exact application payload; raw capture contains none of the test sentinel, selected fingerprint, target, receiver certificate DER, SPKI DER, or raw Pelt key |

The raw relay also scanned for `werewolf`, `fang`, `pelt`, `pack`, `wwp1`,
`target`, `sender`, `receiver`, `ack`, and `protocol`: all `NOT_FOUND`. This is
a raw-byte observation, not a claim of traffic-analysis resistance or active
receiver-identity hiding. A participating TLS client can obtain the server
certificate/Pelt public identity. IP addresses, ports, timing, sizes, and
ordinary TLS handshake metadata remain observable. Private Pelt material,
session secrets, exporter bytes, and key logs were not captured or persisted.

Old plaintext/insecure TCP peers fail the new TLS handshake. The historical
interoperability matrix explicitly confirms old/old still works in its
isolated legacy fixture, while old/new and new/old reject. No insecure
compatibility fallback was added. The Python receiver-binding fixture now
records exact selected-peer receiver trust and unchanged Stage 10 application
transcripts.

## Gates, dependency, and startup fixture

`tokio-rustls = { version = "=0.26.5", default-features = false,
features = ["ring"] }` provides the async TLS stream adapter; its `ring`
feature matches the existing selected provider. Cargo.lock adds exactly this
one package and its direct werewolfd dependency. Existing rustls 0.23.40,
rustls-webpki 0.103.13, Quinn 0.11.9, rcgen 0.13.2, and ed25519-dalek 2.2.0
versions did not change; no new cryptographic provider or parser was added.
The Cargo.lock SHA-256 is
`c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17`.

- `cargo fmt --check`: PASS.
- `cargo clippy --workspace --all-targets`: PASS (pre-existing warnings).
- `cargo test --locked --offline --workspace`: 138 passed, 0 failed, 0 ignored;
  `werewolf-core` 21 unit + 12 integration, `werewolfd` 105. Historical 132
  plus six new TCP tests; no tests removed or ignored. Existing filesystem
  safety tests require the approved unsandboxed temporary-directory ownership.
- `python3 -B -m unittest discover -s tests/integration -p 'test_*.py'`:
  64 passed.
- `python3 -B tests/integration/run.py`: 12/12 passed, including the
  50 MiB encrypted-TCP SHA-256 transfer, Stage 11A fallback matrix, raw
  targets, and daemon restart/profile restoration.
- `git diff --check`: PASS.

The acceptance fixture's first boot has no Pelt: both secure listeners require
a runtime certificate, so the fixture no longer probes TCP readiness before
it calls `pelt.init` and restarts. On subsequent boots with persisted Pelt,
it checks TCP readiness and exercises both network transports normally. The
initial `pelt.init` still requires a daemon restart to bring up the secure
listeners; this is an existing Stage 12 listener-lifecycle limitation, not a
change to QUIC behavior in this checkpoint.

Existing untracked Stage 12B/C proof artifacts were preserved and excluded
from the commits. Their prior secret audit remains applicable; this checkpoint
does not modify them or record runtime secrets.
