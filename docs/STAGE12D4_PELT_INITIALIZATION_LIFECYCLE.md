# Stage 12D.4 — Pelt initialization and secure-listener lifecycle closure

`STAGE12D4_PELT_INITIALIZATION_LIFECYCLE = COMPLETE`.

Before this change, startup derived `RuntimeTlsIdentity` only after loading an
existing `pelt.json`. The TCP and QUIC listener tasks were spawned by `main`,
observed no identity on a fresh Den, returned `handshake rejected`, and were
not restarted when `pelt.init` later created the Pelt. The control socket
remained available, but secure listeners were absent until process restart.

`pelt.init` now generates a candidate Pelt, validates it, derives and checks
its ephemeral runtime certificate before persistence, and then uses the
existing durability coordinator to commit `pelt.json`. Only after durable
success does one state-lock publication install `pelt` and
`runtime_tls_identity` together and mark Pelt ready. Persistence errors,
including `IndeterminateAfterRename`, publish neither value and retain the
existing storage-degraded behavior; no rollback or presumed operational
identity is introduced.

Each owned secure listener task now waits on the daemon state's
`tls_identity_ready` notification while no identity exists. After atomic
publication, `pelt.init` notifies both tasks. They then construct their normal
process-lifetime TLS acceptor/configuration and bind exactly once. Existing
Pelt startup still derives one identity during startup, and repeated
`pelt.init` remains `ALREADY_INITIALIZED`; no rotation or second identity is
possible. Silver remains independent: publishing an identity does not unlock
or authorize forwarding.

The acceptance fixture now checks TCP readiness after same-process
`pelt.init`; the listener task wakes and binds without an identity-driven
restart. The subsequent fixture restart is only its established state-loading
step for Pack and target-policy files. Existing transport wire formats,
authentication, replay, target policy, fallback, authority, and QUIC behavior
remain unchanged.

The lifecycle test `listener_waits_for_same_process_pelt_publication` proves a
fresh no-Pelt state is initially unbound and becomes TLS reachable after
coherent in-process publication and notification. The existing production
QUIC/TCP receiver-authentication, authority/Silver, replay, and target tests
continue to pass. Full gates report 139 Rust tests (138 entering this stage
plus one lifecycle test), 64 Python tests, and 12/12 acceptance scenarios.

No dependency or provider changed. `tokio-rustls` remains 0.26.5,
`rustls-webpki` remains 0.103.13, and Cargo.lock SHA-256 remains
`c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17`.
The permitted Stage 12B/C artifacts remain untracked and contain no private
seed, TLS key, PKCS#8, traffic secret, exporter value, or key log.
