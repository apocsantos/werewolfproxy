# v1 RC cryptographic inventory

| Primitive / component | v1 purpose |
| --- | --- |
| Ed25519 | Long-term Pelt identity, sender OPEN signatures, and receiver acknowledgements |
| X25519 | Ephemeral TCP session agreement; low-order/all-zero peer inputs reject |
| BLAKE3 | Stage10 session-key derivation and QUIC exporter channel binding; Stage15B local consistency commitments use separately domain-separated inputs |
| ChaCha20-Poly1305 | Authenticated encryption for the Stage10 data-frame construction |
| TLS 1.3 / rustls | Secure transport and receiver certificate authentication through exact-SPKI Pelt binding |
| Quinn / QUIC | Secure datagram transport; completed-TLS exporter binds QUIC OPEN to its connection |
| SHA-256 | Stage24 node ID: `SHA-256("werewolf/node-id/v1\0" ‖ Ed25519 public key)` |
| Ed25519 (contact records) | Stage24 signed contact records, domain `werewolf/contact/v1\0`, verified against the record's own key and pinned to it for TLS |
| Ed25519 (RPC) | Stage24 RPC requests and results (`ping`, `find_node`, Stage25 `services`, Stage29 `governance`), transcript domains `fang.quic.rpc/v1`, `fang.quic.rpc.result/v1`, `fang.tcp.rpc/v1`, `fang.tcp.rpc.result/v1`, distinct from every OPEN/ACK domain |
| Ed25519 (governance) | Stage28–30 statements signed by at least 7 of 10 governance keys under `werewolfproxy/governance/v1/sign\0`; the Stage30 key check under its own domain `werewolfproxy/governance/v1/key-check\0` |
| BLAKE3 (governance) | Stage28 statement hash, domain `werewolfproxy/governance/v1/hash\0`, chaining each statement to the previous one |

The implementation composes established libraries and protocol constructions;
the project does not claim a formal proof of its custom transcript, KDF, replay,
or framing composition. Exact Stage10 transcript behavior and Stage12 TLS
identity rules remain the compatibility source. Secret-handling and
zeroization limits are described in Stage14; a privileged live host can still
inspect process memory.

No post-rc.1 stage changed a Stage10 transcript, the KDF, AEAD, framing or
the TLS identity rules; Stage26 routing nests the unchanged encrypted-TCP
session inside another session and adds no construction.

The RC1 lockfile resolves rustls 0.23.45 and rustls-webpki 0.103.15. Stage20S
records why rustls was updated and the exact patched-handshake characterization.
