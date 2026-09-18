# WerewolfProxy v1 RC wire contract

This is a compatibility freeze for the accepted secure forwarding transports.
It records the Stage10/12 behavior; it does not define another protocol.

## Secure transport generations and authentication

The accepted application generations are TCP fang-tcp-v3 and QUIC
fang-quic-v3. There is no descriptive product banner, capability negotiation,
or version/cipher hint exchange. TCP uses TLS 1.3 to authenticate the receiver
before sending Werewolf OPEN semantics. The receiver certificate public key is
bound to the expected Pelt Ed25519 identity using exact SPKI validation; no CA
or hostname substitution is used as the peer identity rule.

Stage20S updates rustls to 0.23.45 and rustls-webpki to 0.103.15. The patched
TLS implementation rejects handshake messages received at the wrong
encryption level across the ServerHello key transition. This changes handling
of an invalid TLS handshake only; the Werewolf application wire format and
valid OPEN/ACK/frame semantics remain frozen.

The sender then proves possession of its Ed25519 key in a signed OPEN. The
receiver binds the request to its identity and request fields. TCP uses a fresh
receiver challenge and client ephemeral X25519 public key; QUIC binds the OPEN
signature to exporter material from the completed TLS connection. ACK identity
and echoed request fields are checked against the retained OPEN. OPEN is not
accepted as application authorization through QUIC 0-RTT.

The transport/channel binding, random challenge/nonce and signature stop a
captured OPEN/ACK from being transplanted to another connection. Replay state
is bounded and connection-scoped according to Stage10/16. This does not stop a
live relay to the intended receiver or a trusted peer from signing a new
request.

## Authorization, publication and frames

Successful sender authentication is followed by Pack membership validation,
exact target-policy authorization, endpoint resolution where applicable, and
target connection. Authority is published only after the authenticated OPEN,
authorization and target connection path succeeds. Pack membership alone is
not target authorization.

Encrypted frame ciphertext is bounded at 4,096 bytes before allocation. OPEN
target text is bounded to 512 UTF-8 bytes and validated before resolution.
Other handshake field encodings, transcript domains, KDF, AEAD, frame counters
and response fields remain the Stage10 contract. They are not renegotiated.
Stage13B retains one outer TCP write per encrypted frame, which affects local
write morphology but adds no transcript field or application framing rule.
The exact signed field encodings and frame/KDF definitions remain recorded in
[Stage10 replay hardening](STAGE10_REPLAY_HARDENING.md); that certification is
part of this freeze and this overview does not supersede it.

## Fallback and time/resource behavior

The default secure selector (secure/strict) tries QUIC secure, encrypted TCP
secure, then fails. It never silently selects plain TCP. Compatibility permits
plain only when explicitly allowed for that invocation; LEGACY preserves the
older explicitly weaker selection behavior. These are local choices, not
remote negotiations.

The application handshake is subject to finite deadlines and resource caps.
Timeout or saturation can close incomplete/abusive sessions earlier. Valid
admitted sessions retain the same wire semantics. Stage16 documents exact
compiled limits. QUIC has no application-authorization 0-RTT path.

## Wire opacity

Authenticate everything, authorize everything, reveal nothing unnecessary.
The guarantee concerns unnecessary semantic plaintext in protocol responses;
it is not traffic-flow confidentiality. Source/destination IP addresses, ports,
TCP versus QUIC, packet sizes, volume, timing, TLS/QUIC fingerprint
characteristics and traffic-analysis relationships remain observable. Stage13
reduces avoidable implementation fingerprints but does not hide traffic.
