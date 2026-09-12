# Stage 12C.1 — Quinn Pelt receiver authentication

`STAGE12C_QUINN_AUTH_PROOF = COMPLETE` for this disposable integration proof.

The real UDP loopback probe passes. The locked Quinn harness builds and its
three cases pass. The receiver-authentication construction from Stage 12B is
preserved; no production transport or protocol changes are made.

## Executed results

| Requirement | Result |
| --- | --- |
| UDP environment | **PASS**: bind, send, and receive on ephemeral IPv4 loopback ports |
| Quinn harness build | **PASS**, locked/offline; formatting and Clippy with warnings denied also pass |
| Correct receiver A | `QUINN_CORRECT_RECEIVER PASS` |
| Exact sentinel recovery | **PASS**: server receives all 235 bytes exactly; client receives the exact echo |
| Wrong receiver B | **REJECT**: exact-SPKI mismatch, `ApplicationVerificationFailure` |
| B application stream open count | **0** |
| Another Pack peer C | **REJECT**, application stream open count **0**; A and C both pass actual Pack-record validation |
| Fake receiver B/C application disclosure | `OPEN_SENTINEL_COUNT=0`, `TARGET_SENTINEL_COUNT=0`, `SENDER_ID_SENTINEL_COUNT=0`, total application bytes **0** |
| Verifier | Certified Stage 12B verifier block copied byte-for-byte; real rustls TLS 1.3 signature verification |
| 0-RTT | `AUTHORIZATION_DATA_IN_0RTT=NO`; client early data disabled, server early-data limit zero, normal handshake await only |
| Resumption | `CLIENT_RESUMPTION=DISABLED`, `SERVER_TICKETS=DISABLED`; no resumed path used |
| Empty ALPN | `QUIC_NO_ALPN=PASS` with both ALPN lists empty |
| Temporary fallback ALPN | Not required; none selected |
| Stage 10 exporter | `EXPORTER_CLIENT_SERVER_MATCH=YES` on A |
| Raw UDP payload byte scan | All requested semantic and exact searches **NOT_FOUND** in the recorded run |
| Passive Initial decryption | **PASS**, using public salt/DCID only; ClientHello contains no SNI, ALPN, PSK, or early-data extension |
| Genuine blocker | **None** |
| Repository | Branch `codex/stage12-hide-wire-opacity`, HEAD `3d2a57de968ac74b1155104c1e087b3a6f289315`; additions remain uncommitted |

[Machine evidence and raw datagrams](../tests/stage12_quinn_auth/evidence.json)
record the final executed run, exact errors, event order, callback counters,
public identity fixtures, application sentinels, passive ClientHello results,
source hashes, and tool versions. The executable exits unsuccessfully if a
required assertion fails. It emits a completed report only after all cases
pass. Permission-denied socket creation instead emits
`QUINN_NETWORK_ENVIRONMENT_BLOCKED` and exits without further network work.

## Exact identity and trust construction

[The standalone harness](../tests/stage12_quinn_auth/src/main.rs) uses locked
Quinn 0.11.9, quinn-proto 0.11.15, quinn-udp 0.5.14, rustls 0.23.40,
rustls-webpki 0.103.13, ring 0.17.14, rcgen 0.13.2, and Tokio 1.52.3.
It depends on the existing `werewolf-core` Pelt/Pack helpers without changing
them. Its Cargo workspace and lockfile are separate from production.

Each identity comes from `pelt::generate_identity()` and passes
`state_validation::identity()`. The 32-byte Ed25519 seed is encoded as canonical
PKCS#8 and imported into rcgen. Runtime certificate generation must produce an
SPKI exactly equal to canonical Ed25519 SPKI wrapping that Pelt's public raw32.
Both rcgen's key SPKI and webpki's parsed certificate SPKI are asserted equal.
Each Pelt also signs and verifies a possession challenge using the production
Pelt helpers.

A/B/C are fresh disposable identities. The earlier Stage 12B run retained no
private seeds, so A is a fresh instance of the certified construction, not a
recovery or reuse of that earlier run's private key. This harness persists only
public identities and test application sentinels, never seeds, private PKCS#8,
TLS traffic secrets, or exported keying material.

Both A and C are represented as `TrustLevel::Packmate` records with their actual
public keys/fingerprints. `state_validation::pack()` accepts the collection.
The selected receiver is A; only A's canonical SPKI is passed to the client
verifier. The verifier never receives the Pack collection. C therefore remains
unacceptable as A despite being an independently valid Pack identity.

[The verifier module](../tests/stage12_quinn_auth/src/verifier.rs) preserves the
entire certified verifier block byte-for-byte, including its real DER parser,
canonical Ed25519 check, exact SPKI comparison, and genuine
`rustls::crypto::verify_tls13_signature` call. The block SHA-256 is
`30cacc2128d0a1f828984c248a0f7a7181f7776d2db47d586bcbc8268eb9f005`.
The additional wrapper only constructs it and exposes audit observations.

The client builds TLS 1.3-only rustls configuration with that verifier, then
converts it through `QuicClientConfig::try_from`. The server builds its config
from the matching certificate/private key and converts it through
`QuicServerConfig::try_from`. The proof uses neither a skip verifier nor CA,
hostname, TOFU, or fingerprint-only acceptance. The certificate DNS name
`runtime.invalid` differs from the IP ServerName used for the connection.

On A, the recorded verifier has one accepted exact-pin check and one successful
genuine TLS 1.3 CertificateVerify check with the correct server context. B/C
each reach the certificate callback once and fail at the exact pin comparison;
neither reaches CertificateVerify. No TLS 1.2 callback occurs.

## Application boundary, early data, and resumption

All three cases run the same application path. Only expected-result assertions
differ after the connection attempt. The code does not suppress application
streams based on knowing that a fixture is negative.

The successful client event sequence is:

1. Start normal `Connecting.await`.
2. Await returns an authenticated connection; assert the pin/signature audit.
3. Increment `application_stream_open_count` immediately before `open_bi()`.
4. Open the stream, then submit the application payload.

For B/C, the only events are await started and await rejected. Stream creation
is never reached. Each payload contains unique OPEN, target, and sender-identity
sentinels incorporating a fresh 256-bit random token. The server observer uses
normal handshake await and records every accepted bidirectional stream's
complete payload. B/C terminate during the handshake, with zero streams and
no application payloads. A receives the entire 235-byte payload and echoes it.
Positive disclosure counts are one for each sentinel, after authentication;
negative disclosure counts are zero.

Actual rustls settings passed into Quinn are:

- Client: `enable_early_data=false` and `Resumption::disabled()`.
- Server: `max_early_data_size=0`, `send_tls13_tickets=0`,
  `NoServerSessionStorage`, and an asserted disabled ticketer.
- Neither endpoint uses an early connection API; the client never calls
  `into_0rtt()` and the server awaits the normal handshake.

Each case uses new endpoint/configuration instances. Public Initial decryption
independently confirms there is no offered PSK or early-data extension.
Successful certificate and CertificateVerify callbacks establish a full
authentication path on A. No resumed path is used. This tests the configured
full-handshake path, not a separate malicious 0-RTT injection campaign or a
production resumption policy.

## ALPN and Stage 10 exporter

The first attempted ALPN configuration is empty on both sides, and it succeeds.
Quinn does not require ALPN in this locked harness configuration. No temporary
ALPN is necessary, and no production ALPN is selected. The IP ServerName yields
no SNI; the server handshake observation and passive ClientHello parse agree.

Both authenticated endpoints successfully call
`Connection::export_keying_material` for 32 bytes with the unchanged inputs:

```text
label:   EXPORTER-WerewolfProxy-Fang-QUIC-v3
context: werewolfproxy/fang-quic-v3
```

The results match exactly. Only the equality result is recorded; the bytes
themselves are not logged. These strings are local exporter inputs and are not
sent as plaintext by the exporter API. Both exact raw scan patterns are also
`NOT_FOUND`. Production Stage 10 exporter construction is untouched.

## Raw datagrams versus passive QUIC observation

A local UDP relay records every UDP payload it receives in both directions,
then forwards it unchanged. These are datagram payload captures, not fabricated
TLS buffers or full IP/UDP-header packet captures. The evidence includes their
exact hex bytes and direction, permitting independent rescanning.

The **RAW DATAGRAM BYTE SCAN** performs case-insensitive literal searches for:

```text
werewolf fang pelt pack wwp1 target sender receiver ack protocol
```

It also performs exact searches for the actual server's test fingerprint,
Pelt raw32, certificate DER, SPKI DER, the expected receiver A's corresponding
public values, the application/OPEN/target/sender sentinels, and both exporter
inputs. All are `NOT_FOUND` in the final recorded three-case run. Search results
include match locations if any are found in a future run; short random matches
are not automatically interpreted as protocol semantics or made into gates.

The **PASSIVE QUIC OBSERVER AFTER STANDARD INITIAL DECRYPTION** is implemented
separately in [passive.rs](../tests/stage12_quinn_auth/src/passive.rs). It uses
the public QUIC v1 Initial salt and original client destination connection ID,
HKDF, header-protection removal, and authenticated AES-128-GCM decryption. It
reassembles CRYPTO bytes and parses ClientHello without endpoint secrets or a
TLS key log. Decryption retains the original DCID when subsequent packets use
the server's new connection ID. Every inspected Initial passes AEAD validation.
This small decoder is scoped to this harness's QUIC v1 packets, not arbitrary
untrusted packet captures or every QUIC version.

In all three cases it observes TLS 1.3, empty ALPN, no SNI, and no PSK/early-data
extensions. Cipher suites, extension choices/order, key shares, and QUIC
transport-parameter bytes remain available in the public ClientHello. If
SNI/ALPN were offered, their ClientHello bytes would likewise be passively
recoverable. Empty ALPN and SNI remove those fields in this profile; they do not
make QUIC Initial opaque. Naive raw byte absence is not a full QUIC opacity
claim.

The certified boundary is that the tested wrong/fake receivers cannot
authenticate as selected Pelt A, so the client never opens an
authorization-bearing application stream to them. This does not establish
active receiver identity hiding, traffic-analysis resistance, QUIC fingerprint
camouflage, IP/port hiding, timing hiding, or size hiding.

## Reproduction and repository scope

From the repository root:

```sh
cargo run --locked --offline --manifest-path tests/stage12_quinn_auth/Cargo.toml --target-dir target/stage12-quinn-auth
cargo fmt --manifest-path tests/stage12_quinn_auth/Cargo.toml -- --check
cargo clippy --locked --offline --manifest-path tests/stage12_quinn_auth/Cargo.toml --target-dir target/stage12-quinn-auth -- -D warnings
```

Use `cargo run` to execute the assertion-driven proof. Offline execution needs
the locked dependencies cached. Socket PermissionDenied is an environment
blocker and does not certify an architectural failure; the harness stops
network execution without attempting privilege escalation or a sandbox bypass.

The Stage 12B certification document and evidence remain present and unchanged.
Production Stage 10 exporter construction, OPEN/ACK, replay, Stage 9
authorization, Stage 11C authority, production QUIC transport, and root Cargo
files remain unchanged. No commit was made and no Codex internal/session files
were accessed. Work stops at this proof.
