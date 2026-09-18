# Stage 21 — RC1 audit and freeze

**Version:** `1.0.0-rc.1`  
**Scope:** Linux x86_64, `x86_64-unknown-linux-gnu`  
**Result:** `STAGE21_RC1_AUDIT_FREEZE = CERTIFIED`

Stage 21 resumed the interrupted RC audit from its retained history. It did
not restart the stage or change a frozen application interface. The only
runtime dependency change in the history was the separately certified Stage20S
rustls security remediation. Stage21 release edits and this certification are
documentation, tests, and build/package metadata.

## Resumption and history

- Stage21 advisory-stop commit: `8a15958`.
- Stage20S certified baseline: `c2f6f2681a72de53187dc1008eec29d1a6744b81`.
- Stage20S release-input/code HEAD: `fc957a0`.
- Stage21 was resumed on the existing `codex/stage21-rc1-audit-freeze`
  branch. The branch was fast-forwarded from `9b3c1de` to the certified
  Stage20S HEAD; no history was rewritten.
- Existing Stage21 commits retained as ancestors: `a092fc7`, `e3926f9`,
  `8a15958`. The packaging-only fix `9b3c1de` and Stage20S commits
  `bca5ec0`, `86bd02a`, `5befb1c`, `5f2e3b3`, `fc957a0`, `c2f6f26` are also
  retained.
- Stage21 release-input commits: `d77e324`, `e6ecd71`, `db16406`.
- Final Stage21 release-input commit: `db164061690254b85feb6c27af5e67d56ac535b9`.
- Production-code provenance recorded by the release builder:
  `48b9f9ad0a37b91645fe7ed939892a4b8fef60dd`.
- Final certification HEAD is the commit containing this report and is the
  exact target of the annotated `v1.0.0-rc.1` tag. The tag target was verified
  after that commit was created; it was not pushed.

`8a15958` was inspected at the original advisory stop. Its only source change
was `packaging/make-release.sh`, binding version provenance to the production
code commit. It changes release/build metadata only and does not alter runtime
security semantics.

## Advisory, dependency graph, SBOM, and licenses

Stage21's prepublication advisory gate found RUSTSEC-2026-0285 against
rustls 0.23.40. No RC was tagged or published with that version. Stage20S
preserved the reproducer and certified rustls 0.23.45, which rejects the
malformed TLS 1.3 encryption-epoch fixture. The permanent regression is
`crates/werewolfd/src/tls_identity.rs::tests::tls13_plaintext_encrypted_extensions_after_server_hello_are_rejected`.

The final Cargo.lock has SHA-256
`fbc16d90daaf58ba7f3f32f10615f8521103a5c019f98132ee37c36928458704` and 224
locked dependencies. The graph contains rustls 0.23.45 and rustls-webpki
0.103.15. `werewolfd` directly pins both (`=0.23.45` and `=0.103.15`);
rustls is also reached through tokio-rustls 0.26.5 and Quinn 0.11.9 /
quinn-proto 0.11.15. The exact lockfile is authoritative.

The final cargo-audit run used the RustSec advisory database at
`2026-09-18T11:06:23+02:00` (database commit
`2b34578f89884736e0fcbd42f7ba8d6b10b4a0ce`). It scanned all 224 locked
dependencies: **0 vulnerabilities, 0 warnings**. RUSTSEC-2026-0285 is absent.
This is a dated audit result, not a claim about future advisories.

The deterministic SBOM contains 224 components, with 154 in the Linux release
closure. `packaging/THIRD_PARTY_NOTICES` contains 128 deduplicated shipped
license texts. The generator was run twice after the final dependency update;
both outputs matched each other and the committed files byte-for-byte. Every
registry package has a license declaration or license-file reference; the
generator fails closed when that information is missing.

| Inventory | SHA-256 |
| --- | --- |
| `Cargo.lock` | `fbc16d90daaf58ba7f3f32f10615f8521103a5c019f98132ee37c36928458704` |
| `packaging/SBOM.json` | `2e6a20aca03ff0d122f82f2bcf24e8fe12ffd5e5e8c8e084daa3c3a5a2437b47` |
| `packaging/THIRD_PARTY_NOTICES` | `94bf3a374d019c2e49de864f0a463356692cd1c6193b104589d94a67bb1b93c5` |

Earlier Stage11–12 documents retain their historically correct versions and
lockfile digests. `docs/INDEX.md` identifies them as historical and names the
current Stage21 graph. The release notes, README, CHANGELOG, SECURITY, SBOM,
notices, and manifest describe the patched graph. Their references to 0.23.40
or 0.103.13 explicitly describe the superseded vulnerable baseline and its
remediation; none presents those versions as current.

## Frozen interfaces and Stage13 observations

| Interface | Freeze |
| --- | --- |
| `WIRE_V1_RC1` | FROZEN |
| `PERSISTENCE_V1_RC1` | FROZEN |
| `CLI_V1_RC1` | FROZEN |
| `LINUX_SERVICE_V1_RC1` | FROZEN |

`STAGE21_WIRE_FORMAT_CHANGE = NONE`.  
`STAGE21_PERSISTENCE_FORMAT_CHANGE = NONE`.  
`STAGE21_CLI_SURFACE_CHANGE = NONE`.  
`STAGE21_RUNTIME_FEATURE_CHANGE = NONE`.

The Stage20S traffic characterization compared the prior Stage13 30-sample
baseline with five patched samples per regular workload on both transports.
Valid sessions completed. Patched rustls measurably changed upstream TCP TLS
record grouping (for example, median record counts changed from 43 to 27 for
interactive traffic and 3,027 to 1,519 for a 1 MiB transfer); broad QUIC
datagram totals stayed similar, with some directional byte-count changes.
Werewolf application messages did not change. These are TLS implementation
observations, not a Werewolf wire redesign or traffic-flow confidentiality
claim. Stage13 design was not reopened.

## Security and release audits

- **Receiver authentication:** generated Pelt certificate identity and exact
  SPKI pinning pass for TCP and QUIC. Genuine TLS 1.3 CertificateVerify passes;
  wrong-key, bit-flipped, malformed-certificate, and unrelated-SPKI cases are
  rejected before forwarding. The selected Pack peer is bound to the
  verifier. There is no CA or hostname substitution, TOFU, TLS early-data
  authorization, or session resumption; TLS 1.3 remains the only version.
- **TCP:** tokio-rustls handshake, exact-SPKI, v3 OPEN/ACK, layered X25519
  session setup, target authorization, forwarding, revocation, and Silver
  regression tests pass.
- **QUIC:** Quinn authenticated connection, exact-SPKI, exporter and Stage10
  channel binding, OPEN/ACK, authorization, forwarding, revocation, and Silver
  regression tests pass.
- **Malformed TLS/resource bounds:** the wrong-encryption-epoch regression,
  partial and truncated handshakes, unexpected messages, wrong certificate,
  invalid CertificateVerify, unknown peer, stalls/timeouts, malformed TCP
  and QUIC input, pre-auth admission, permit release, and valid reconnect
  tests pass. No hostile case panicked or forwarded application data.
- **Stage13–16:** Stage20S's representative traffic characterization is
  recorded above. Pre-auth limits, TLS stall timeout, malformed recovery,
  permit release, and reconnect-after-hostile-input pass.
- **Unsafe audit:** no `unsafe` block, function, impl, trait, or static was
  found in shipped production Rust sources.
- **Panic-surface audit:** the shipped production scan found only existing
  internal mutex-poison unwraps, process-startup signal registration expects,
  one bounded transcript conversion expect, and CLI serialization/dispatch
  invariants. No remote-input-triggerable panic was found; malformed TLS and
  hostile-input suites passed. Test-only and non-shipped lab binaries are not
  production panic surfaces.
- **Crypto formatting/logging:** Stage14 and Stage20S audits remain
  applicable. Pelt Debug is redacted; runtime TLS identity has no Debug or
  Serialize implementation; TLS errors are categorical. No private Pelt
  seed, PKCS#8, X25519 private/shared secret, session key, exporter, traffic
  secret, key log, or raw protected payload is logged or added to fixtures.
  The new regression generates identity material in memory only.
- **Wire-string audit:** Stage21 adds no application wire strings or
  serialization fields. The rustls update rejects only the invalid TLS 1.3
  encryption-level fixture; application framing and valid OPEN/ACK behavior
  remain frozen.
- **Release-blocker review:** no new authentication, authorization, receiver
  binding, replay, channel-binding, Silver, revocation, target-policy,
  opacity, secret-handling, persistent-state, resource-bound, lifecycle, or
  release-integrity blocker remains open.
- **Nonclaims retained:** no arbitrary DoS immunity, no traffic-flow
  confidentiality, no external complete-state rollback freshness, and no
  certification beyond Linux x86_64 / glibc 2.41. Stage20 did not directly
  execute netem packet loss/jitter/reordering, direct ENOSPC, read-only
  filesystem injection, host-clock changes, or systemd restart chaos.

## Final release inputs and reproducibility

The final RC was rebuilt twice from clean, isolated target and output
directories using the locked, offline release procedure. Both builds used
`SOURCE_DATE_EPOCH=1778563200`, release-input commit
`db164061690254b85feb6c27af5e67d56ac535b9`, production-code commit
`48b9f9ad0a37b91645fe7ed939892a4b8fef60dd`, rustc 1.96.0, target
`x86_64-unknown-linux-gnu`, and Debian 13.6 / glibc 2.41. The daemon, CLI,
systemd unit, release manifest, inner and outer checksum files, extracted
archive contents, and compressed archive were byte-identical between builds.

The measured daemon and CLI digests equal Stage20S's historical binary
digests because their production code provenance and runtime application code
are unchanged. They were freshly produced and compared in the two Stage21
builds; the Stage20S archive hash was not reused. The Stage21 archive has its
own newly measured digest.

| Final artifact | SHA-256 |
| --- | --- |
| `werewolfd` | `ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4` |
| `werewolfctl` | `c9c5e5cf17350b675927efd419e7bb970c3e136ff79977ce8101357e07f74818` |
| `systemd/werewolfd.service` | `97017876b9be39112d9f597e0b3b32ed9a6c474fa59b65ddf0ac910d5eed5a6c` |
| `RELEASE-MANIFEST.json` | `5b0e174cedbede5501b8c772282094ee1050e83294ac7e92f912149776a909c8` |
| Archive `SHA256SUMS` | `16e5b4790acb60fe602a48f7459b2016015d10f416483678652bd8496978b5d4` |
| Archive `werewolfproxy-1.0.0-rc.1-linux-x86_64.tar.gz` | `e2d372739743b31359fc115d38f36c3db1c86ebec8afab026b244811c5cacea4` |

The archive contains 16 expected files. Its manifest records version
`1.0.0-rc.1`, the release-input and production-code commits, source epoch,
toolchain, platform, Cargo.lock hash, rustls versions, and daemon/CLI/service
hashes. Inner `SHA256SUMS` verifies each release file; outer `SHA256SUMS`
verifies the archive.

## Final archive tests and gates

All archive-facing tests used binaries extracted from the final Stage21
archive, not checkout binaries:

- Archive checksum verification, extraction, installer staging, archive
  inventory, no implicit identity creation, reinstall, checksum-failure
  safety, unsafe staging-path rejection, execution without a checkout, and
  uninstall: PASS.
- Corrected written QUICKSTART, with a secure umask and two isolated local
  peers: Pelt/Pack setup, migration, Silver, target allow, Fang creation,
  `status`, `doctor`, TCP forwarding, and QUIC forwarding all PASS.
- QUICKSTART 50 MiB TCP and QUIC round trips both matched SHA-256
  `67409f81bfa4dfb1b8eadd3b31d5e6c097aac55d99df0d5cd5b327fd12119c6f`.
- Populated Den uninstall/reinstall preserved every file, exact Pelt bytes,
  Pelt fingerprint, and Silver-open manifest state.
- Stage17 lifecycle: PASS. Stage18 CLI provisioning/revocation: PASS.
- Stage19 release archive harness: PASS. Stage18-to-RC upgrade preserved
  Pelt, Pack, targets, Silver and Fang intent. Downgrade to the compatible
  Stage18 binary baseline preserved state and forwarding: PASS. The historical
  Stage18 binaries were built only for this local characterization and were
  not included in the RC.
- Stage20 CI chaos against the final archive: PASS in 75.944 seconds, seed
  `20260918`. It completed 20 TCP reconnects, 20 QUIC reconnects, 20 mixed
  cycles, 3 peer restarts, 5 SIGKILL recoveries, 64 malformed TCP inputs, 64
  malformed QUIC datagrams, pre-auth saturation/recovery, Silver and revoke
  on both transports, one 50 MiB TCP transfer, one 50 MiB QUIC transfer, and
  64 hash checks.
- Full source gates at the final release-input tree: `cargo fmt --check`
  PASS; `cargo clippy --workspace --all-targets` PASS with five existing
  warnings reviewed; `cargo test --locked --offline --workspace` PASS,
  188 tests (core 22, daemon 154, integration targets 12).
- Python integration unit tests: 64 PASS. Acceptance: 12/12 PASS.
- `git diff --check`: PASS. Working tree was clean at the final certification
  commit.

No previously certified test was removed, ignored, filtered, or weakened.

## Production-change inventory from Stage20 parent

Relative to certified Stage20 parent
`2abcd63b031b77cef99315742ae14d0192720c2a`:

- **Stage21 documentation/contracts:** `CHANGELOG.md`, `KNOWN_LIMITATIONS.md`,
  `QUICKSTART.md`, `README.md`, `RELEASE_NOTES_1.0.0-rc.1.md`, `SECURITY.md`,
  `docs/INDEX.md`, the V1 wire/persistence/CLI/Linux-service/threat/security/
  cryptography contracts, and this Stage21 certification.
- **Stage21 packaging/build metadata:** `packaging/SBOM.json`,
  `packaging/THIRD_PARTY_NOTICES`, `packaging/generate-sbom.py`,
  `packaging/make-release.sh`, `packaging/release.env`, `packaging/INSTALL.md`,
  and `tests/stage19_release/release.py`. Fixes preserve the interrupted
  Stage21 packaging change, bind binary version provenance, normalize notices,
  assert patched dependency metadata, and include Stage20S security history.
- **Stage20S dependency/security remediation:** `crates/werewolfd/Cargo.toml`,
  `Cargo.lock`, both Stage12 TLS/Quinn authentication harness manifests,
  lockfiles and version evidence, and the permanent wrong-epoch TLS regression
  in `crates/werewolfd/src/tls_identity.rs`.
- **No unrelated runtime refactor:** production daemon behavior changes only
  through the certified patched rustls dependency; application wire,
  persistence and CLI formats remain unchanged.

## Stage21 C1–C30 claims

| Claim | Result | Evidence |
| --- | --- | --- |
| C1. Stage21 resumed from the retained checkpoint and preserved history | CERTIFIED | Fast-forwarded existing branch; all listed Stage21/20S commits remain ancestors |
| C2. Stage20S baseline and RUSTSEC remediation are authoritative | CERTIFIED | `c2f6f26`; remediation report and permanent regression |
| C3. Final dependency graph resolves patched rustls 0.23.45 | CERTIFIED | Cargo.lock, direct pin, `cargo tree` |
| C4. Final graph resolves rustls-webpki 0.103.15 | CERTIFIED | Cargo.lock, direct pin, SBOM |
| C5. RUSTSEC-2026-0285 is absent from the dated final audit | CERTIFIED | 224 dependencies; 0 vulnerabilities, 0 warnings |
| C6. SBOM and notices match the final locked graph | CERTIFIED | Two deterministic regenerations; 224 components, 128 license texts |
| C7. License inventory is complete for shipped dependencies | CERTIFIED | Generator license declarations/references and release closure audit |
| C8. Stage21 documents use current dependency facts and preserve history | CERTIFIED | Required docs reviewed; historical versions labeled; INDEX includes Stage20S |
| C9. Nonclaims and certified platform remain accurate | CERTIFIED | KNOWN_LIMITATIONS and Stage20/20S records |
| C10. V1 wire contract is frozen | CERTIFIED | `WIRE_V1_RC1 = FROZEN`; application format change NONE |
| C11. V1 persistence contract is frozen | CERTIFIED | `PERSISTENCE_V1_RC1 = FROZEN`; schema change NONE |
| C12. V1 CLI contract is frozen | CERTIFIED | `CLI_V1_RC1 = FROZEN`; surface change NONE |
| C13. Linux service contract is frozen | CERTIFIED | `LINUX_SERVICE_V1_RC1 = FROZEN` |
| C14. No Stage21 runtime feature or unrelated refactor was added | CERTIFIED | Production-change inventory and release-input diff |
| C15. TLS wrong-encryption-epoch regression is permanently retained | CERTIFIED | Named `tls_identity.rs` test passes on final Rust suite |
| C16. TCP exact-SPKI and TLS 1.3 CertificateVerify remain enforced | CERTIFIED | TLS receiver-auth unit/integration tests and Stage12 evidence |
| C17. QUIC exact-SPKI and TLS 1.3 CertificateVerify remain enforced | CERTIFIED | Quinn auth tests and Stage12C1 evidence |
| C18. Stage10 replay and QUIC channel binding remain intact | CERTIFIED | Full suite and Stage20S transport proof |
| C19. Stage9 exact target authorization remains intact | CERTIFIED | TCP/QUIC live authorization matrices |
| C20. Stage11C authority, Silver and revocation remain intact | CERTIFIED | Rust tests, CLI harness, final chaos on both transports |
| C21. Stage16 pre-auth bounds and malformed recovery remain intact | CERTIFIED | Rust receiver tests and final chaos |
| C22. Unsafe/panic/crypto-formatting/wire-string audits found no release blocker | CERTIFIED | Scoped source reviews, prior Stage14/20S evidence, hostile-input tests |
| C23. Release manifest identifies exact dependency and build inputs | CERTIFIED | Manifest records `db16406`, code commit, epoch, toolchain, target, lock and versions |
| C24. SBOM, notices, manifest and archive checksums are consistent | CERTIFIED | Lock hash and versions match; inner/outer checksums pass |
| C25. Two clean final release builds are byte-reproducible | CERTIFIED | Binaries, metadata, checksums, extracted contents and archive compare equal |
| C26. Final archive installs and works without Git checkout | CERTIFIED | Stage19 archive harness and staged installed-binary run |
| C27. QUICKSTART provisioning, status, doctor, TCP and QUIC pass | CERTIFIED | Final extracted archive, two isolated peers |
| C28. Final TCP and QUIC 50 MiB integrity passes | CERTIFIED | Both round trips match the recorded SHA-256; CI also verifies both |
| C29. Lifecycle, provisioning, release, chaos and full test gates pass | CERTIFIED | Stage17–20, Rust 188, Python 64, acceptance 12/12 |
| C30. Release blockers are closed and final RC tag targets this certification | CERTIFIED | Final review; annotated tag target verified after cert commit |

## Required output register (items 1–67)

1. **Advisory-stop HEAD:** `8a15958`.
2. **Stage20S certified HEAD:** `c2f6f2681a72de53187dc1008eec29d1a6744b81`.
3. **Stage20S release-input/code HEAD:** `fc957a0`.
4. **Remediation:** rustls 0.23.40 → 0.23.45; RUSTSEC-2026-0285 REMEDIATED.
5. **Stage20S lock:** `fbc16d90daaf58ba7f3f32f10615f8521103a5c019f98132ee37c36928458704`.
6. **Stage20S tests:** Rust 188, Python 64, acceptance 12/12; Stage17–20 PASS.
7. **Recovery:** existing Stage21 work preserved; Stage21 resumed, not restarted.
8. **Branch:** `codex/stage21-rc1-audit-freeze`.
9. **Release-input commit:** `db164061690254b85feb6c27af5e67d56ac535b9`.
10. **Final version:** `1.0.0-rc.1`.
11. **Certified platform:** Linux x86_64, `x86_64-unknown-linux-gnu`.
12. **Environment:** Debian 13.6, glibc 2.41.
13. **Toolchain:** rustc/cargo 1.96.0.
14. **Build epoch:** `SOURCE_DATE_EPOCH=1778563200`.
15. **Locked dependency count:** 224.
16. **Linux release closure:** 154 dependencies.
17. **rustls:** 0.23.45.
18. **rustls-webpki:** 0.103.15.
19. **Dependency paths:** direct `werewolfd` pin; also tokio-rustls and Quinn/quinn-proto.
20. **Cargo.lock SHA-256:** `fbc16d90daaf58ba7f3f32f10615f8521103a5c019f98132ee37c36928458704`.
21. **Advisory database:** 2026-09-18 11:06:23 +02:00, commit `2b34578`.
22. **Advisories:** 0 vulnerabilities, 0 warnings.
23. **SBOM:** deterministic, 224 components; SHA-256 `2e6a20aca03ff0d122f82f2bcf24e8fe12ffd5e5e8c8e084daa3c3a5a2437b47`.
24. **Notices:** deterministic, 128 license texts; SHA-256 `94bf3a374d019c2e49de864f0a463356692cd1c6193b104589d94a67bb1b93c5`.
25. **License audit:** PASS; no missing registry license declarations/references.
26. **Security history:** Stage21 found the advisory before publication; Stage20S certified the fix.
27. **Wire freeze:** FROZEN; no Werewolf application wire-format change.
28. **Persistence freeze:** FROZEN; no persistence-format change.
29. **CLI freeze:** FROZEN; no CLI-surface change.
30. **Linux service freeze:** FROZEN.
31. **Runtime feature change:** NONE.
32. **`cargo fmt --check`:** PASS.
33. **Clippy:** PASS; five existing warnings reviewed.
34. **Rust tests:** 188 PASS, 0 failed.
35. **Rust breakdown:** core 22; daemon 154; integration targets 12.
36. **Python integration unittests:** 64 PASS.
37. **Acceptance:** 12/12 PASS.
38. **Stage17 lifecycle:** PASS against final archive binaries.
39. **Stage18 CLI provisioning:** PASS against final archive binaries.
40. **Stage19 archive harness:** PASS against the final archive.
41. **Stage19 upgrade:** PASS; state and Fang intent preserved.
42. **Stage19 downgrade:** PASS; compatible state and forwarding preserved.
43. **Stage20 CI chaos:** PASS against final archive binaries.
44. **Chaos counters:** 20 TCP, 20 QUIC, 20 mixed cycles; 3 restarts; 5 SIGKILL recovery.
45. **Malformed input:** 64 TCP inputs and 64 QUIC datagrams; zero malformed-log growth.
46. **Final QUIC route regression:** PASS, the 10,000-route short-lived local-Fang workload.
47. **Final QUICKSTART:** PASS with two isolated peers from extracted archive.
48. **Install/provision/status/doctor:** PASS using staged installed binaries.
49. **TCP secure forwarding:** PASS.
50. **QUIC secure forwarding:** PASS.
51. **50 MiB TCP SHA-256:** PASS, `67409f81bfa4dfb1b8eadd3b31d5e6c097aac55d99df0d5cd5b327fd12119c6f`.
52. **50 MiB QUIC SHA-256:** PASS, same digest.
53. **Uninstall:** software removed; Den retained.
54. **Populated Pelt:** exact bytes/fingerprint survived uninstall/reinstall.
55. **Silver and manifest state:** preserved and recovered.
56. **Receiver exact-SPKI:** PASS for TCP and QUIC.
57. **CertificateVerify:** valid accepted; invalid/wrong signatures rejected.
58. **TLS modes:** TLS 1.3 only; no early-data authorization or resumption.
59. **Replay/channel binding:** Stage10 guarantees PASS.
60. **Target authorization:** Stage9 exact grants PASS.
61. **Silver/revocation:** Stage11C guarantees PASS in both transports.
62. **Stage16 bounds:** admission, stalls, malformed recovery and permit release PASS.
63. **Unsafe audit:** no unsafe production constructs found.
64. **Panic/secret/wire-string review:** PASS; no hostile-input panic or secret material added.
65. **Release reproducibility:** two clean isolated builds byte-identical.
66. **Binary SHA-256:** daemon `ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4`; CLI `c9c5e5cf17350b675927efd419e7bb970c3e136ff79977ce8101357e07f74818`.
67. **Archive SHA-256:** `e2d372739743b31359fc115d38f36c3db1c86ebec8afab026b244811c5cacea4`; final manifest and inner/outer checksum files verify.

`STAGE21_RC1_AUDIT_FREEZE = CERTIFIED`
