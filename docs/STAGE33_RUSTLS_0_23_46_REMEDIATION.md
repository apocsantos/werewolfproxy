# Stage33 rustls 0.23.46 remediation

Owner decision, 2026-10-29 (Stage33 step 4, option a): remediate the four
rustls advisories reported against `main` before the `1.0.0-rc.2`
candidate audit, as Stage20S did for rc.1. Dependency change only; no
Werewolf code changes.

## The advisories

GitHub Dependabot reported four advisories against rustls 0.23.45, each in
the workspace lockfile, `crates/werewolfd/Cargo.toml` and both Stage12
proof harness lockfiles (16 alerts). All four are fixed in 0.23.46.

| Advisory | Severity | Affected | Exposure in Werewolf on 0.23.45 (code reading) |
|---|---|---|---|
| GHSA-c8m5-c952-6cm4: session `Debug` formatting exposes TLS master secrets and resumption PSKs | medium | `< 0.23.46` | Werewolf formats no rustls connection or session with `Debug` outside tests, but the daemon installs a `tracing_subscriber`, so rustls's own logging could not be excluded: possibly exposed |
| GHSA-h2v8-394m-p7vv: Brotli certificate decompression can allocate up to 1 GiB before authentication | low | `< 0.23.46` | the `brotli` feature is not enabled: not exposed |
| GHSA-w44r-93j7-qv28: an unauthenticated ServerHello poisons the per-name key-exchange record, so later ClientHellos carry only a classical X25519 share | low | `< 0.23.46` | client resumption is disabled (`Resumption::disabled()`), so there is no per-name memory to poison: probably not exposed |
| GHSA-vp88-798v-h67f: ECH rejection can expose buffered application data to the outer server | low | `>= 0.23.44, < 0.23.46` | Werewolf configures no ECH: not exposed |

The exposure column is a reading of the code, not a proof; the
remediation does not depend on it.

## The change

| Dependency | Before | After |
|---|---|---|
| `rustls` in `crates/werewolfd/Cargo.toml` | `=0.23.45` | `=0.23.46` |
| `rustls` in both Stage12 proof harnesses | `=0.23.45` | `=0.23.46` |
| `rustls-webpki` (all three) | `=0.103.15` | unchanged |

`cargo update -p rustls --precise 0.23.46` changed only `rustls` in the
workspace `Cargo.lock` (224 packages, unchanged count); no other
dependency moved and no rustls 0.24 entered. The new `Cargo.lock` SHA-256
is `fc7f0dd54e9e138027a4f389bf2a0f1faf04efc2a0f290284e82459d9fa69c23`.
rustls 0.23.46 is the only rustls on every path: direct, `tokio-rustls`
0.26.5, and Quinn 0.11.9 / quinn-proto 0.11.15 /
rustls-platform-verifier 0.6.2. The Stage12 proof harnesses' report
labels name the new version; the Quinn harness lockfile also records the
`werewolf-core` path version `1.0.0-rc.2` from step 2.

Previous archive hashes are obsolete for the candidate; the release
inputs of step 4 are built on this commit.

## Validation

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -D
  warnings`: pass.
- `cargo test --locked --offline --workspace`: pass (daemon 297, core 94,
  and every other target), including the permanent RUSTSEC-2026-0285
  regression
  `tls_identity::tests::tls13_plaintext_encrypted_extensions_after_server_hello_are_rejected`
  and the TLS identity tests (exact-SPKI pinning, TLS 1.3 only, resumption
  and tickets disabled).
- Stage12 proof harnesses rerun on 0.23.46: the isolated TLS 1.3
  authentication matrix (`stage12_tls_auth`) `PASS`; the Quinn
  authentication proof (`stage12_quinn_auth`) `COMPLETE`, UDP environment
  `PASS`; no failing case in either.
- Stage lab gates (disposable daemons, commit `382d678`): all ten pass.
  `service_gate` (10), `discovery_gate` (12), `service_discovery_gate`
  (7), `routed_fang_gate` (12), `provider_gate` (7), `governance_gate`
  (6), `activation_gate` (4), `sync_gate` (5), `death_gate` (6),
  `auto_sync_gate` (9). The first `service_gate` run passed its ten
  checks but failed its source-preservation check, because this record
  was written and the harness build directories removed while it ran (an
  operator error, not a product result); its rerun on an untouched tree
  passed.

Not rerun for this patch: the Stage13 traffic captures (Stage20S found
that a rustls update can change TLS record grouping; that is an
implementation-level observation, not a Werewolf message change) and the
Stage32 campaigns. The candidate audit's gates (decision 6a) run on the
archive built from this commit.
