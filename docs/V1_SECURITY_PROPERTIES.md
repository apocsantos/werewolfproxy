# v1 security property matrix

| Property | Scope and limit | Certification evidence |
| --- | --- | --- |
| Receiver authentication | TLS 1.3 and exact-SPKI Pelt binding | Stage12; Stage17–20 release harnesses |
| TLS handshake encryption levels | Wrong-epoch TLS 1.3 handshake messages are rejected | Stage20S rustls advisory regression |
| Sender authentication | Ed25519 signed connection-bound OPEN | Stage10, Stage12 |
| Pack membership | Stable fingerprint membership authenticates peer identity, not unlimited authorization | Stage11C, Stage18 |
| Target authorization | Exact Stage9 address/port grants; DNS endpoints are all checked before connect | Stage9, Stage18 |
| Replay resistance | Nonce/challenge and transport/channel binding with bounded replay state | Stage10, Stage16, Stage20 |
| Connection binding | TCP fresh challenge; QUIC completed-TLS exporter | Stage10, Stage12 |
| Revocation | Fence blocks new publication/submission after linearization; prior submitted bytes cannot be recalled | Stage11C, Stage18, Stage20 |
| Silver | ON advances epoch/cancels; OFF requires durable open state before new authority | Stage11C, Stage15B, Stage20 |
| Wire opacity | Unnecessary semantic plaintext is avoided; metadata remains visible | Stage12, Stage13 |
| Secret hygiene | Private key/session material redacted and zeroized where applicable | Stage14, Stage20 |
| Persistent consistency | One current Stage15B generation; partial/mixed restore rejects | Stage15B, Stage18–20 |
| Freshness | Complete historical internally consistent state may still be accepted | Stage15A/B nonclaim |
| Resource bounds | Compiled concurrency, buffer, stream and timeout limits; not DoS immunity | Stage16, Stage20 |
| Lifecycle | Owned tasks, bounded signal shutdown, lock/socket recovery | Stage17, Stage20 |
| CLI provisioning | Supported local operations avoid protected JSON edits | Stage18, Stage19–20 |
| Packaging | Linux x86_64 staging, upgrade and state-preserving uninstall | Stage19, Stage20 |
| Adversarial soak | Recorded bounded campaigns; several chaos profiles were not run | Stage20 and Known limitations |

See individual stage records for test counts, fixture details and exact
preconditions. This table does not expand a tested property beyond its stated
threat model.
