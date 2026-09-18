# v1 RC cryptographic inventory

| Primitive / component | v1 purpose |
| --- | --- |
| Ed25519 | Long-term Pelt identity, sender OPEN signatures, and receiver acknowledgements |
| X25519 | Ephemeral TCP session agreement; low-order/all-zero peer inputs reject |
| BLAKE3 | Stage10 session-key derivation and QUIC exporter channel binding; Stage15B local consistency commitments use separately domain-separated inputs |
| ChaCha20-Poly1305 | Authenticated encryption for the Stage10 data-frame construction |
| TLS 1.3 / rustls | Secure transport and receiver certificate authentication through exact-SPKI Pelt binding |
| Quinn / QUIC | Secure datagram transport; completed-TLS exporter binds QUIC OPEN to its connection |

The implementation composes established libraries and protocol constructions;
the project does not claim a formal proof of its custom transcript, KDF, replay,
or framing composition. Exact Stage10 transcript behavior and Stage12 TLS
identity rules remain the compatibility source. Secret-handling and
zeroization limits are described in Stage14; a privileged live host can still
inspect process memory.

The RC1 lockfile resolves rustls 0.23.45 and rustls-webpki 0.103.15. Stage20S
records why rustls was updated and the exact patched-handshake characterization.
