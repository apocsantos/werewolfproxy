# Stage20S rustls RUSTSEC-2026-0285 remediation

**Status:** `STAGE20S_RUSTLS_2026_0285_REMEDIATION = CERTIFIED`

**Scope:** Linux x86_64, `x86_64-unknown-linux-gnu`; Debian GNU/Linux 13.6,
glibc 2.41; rustc 1.96.0. The candidate product version remains
`1.0.0-rc.1`. Artifacts were built locally for certification and were not
published.

## Recovery and starting state

- Stage21 stopped at its cargo-audit advisory gate on `8a15958`.
- The valid interrupted Stage21 work was retained. Its commits
  `a092fc7`, `e3926f9`, and `8a15958` remain in branch history.
- The packaging-only target-directory fix was preserved separately as
  `9b3c1de` before creating this remediation branch.
- Stage20S started from the preserved checkpoint `9b3c1de` on
  `codex/stage20s-rustls-2026-0285`; no history was reset, rebased, squashed,
  or recreated.
- `8a15958` changes only `packaging/make-release.sh` and binds release
  provenance/build metadata to the production code commit. It does not change
  runtime security semantics.
- No `v1.0.0-rc.1` tag was found or created. No merge, push, or release
  publication occurred.

## Advisory and dependency remediation

The local RustSec advisory database identifies RUSTSEC-2026-0285 (GHSA-2mjx-qc3c-rqvc),
dated 2026-09-14, as a TLS 1.3 encryption-level boundary error. The advisory
marks versions below 0.23.13 unaffected and versions `>=0.23.45` patched; the
affected 0.23 range is `>=0.23.13, <0.23.45`. Rustls 0.23.40 was resolved at
the Stage21 stop. The Stage21 reproduction accepted a plaintext
`EncryptedExtensions` message appended to the plaintext `ServerHello` record
with 0.23.40, while 0.23.45 rejected the same fixture with
`KeyEpochWithPendingFragment`.

`rustls` is both a direct dependency of `werewolfd` and a transitive dependency
through Quinn (`quinn` → `quinn-proto` → `rustls-platform-verifier`) and
`tokio-rustls`. The patched graph has rustls 0.23.45 on all those paths.

| Dependency | Before | After | Reason |
| --- | --- | --- | --- |
| `rustls` in `crates/werewolfd/Cargo.toml` | `0.23` | `=0.23.45` | Pin the patched security version |
| `rustls-webpki` in `crates/werewolfd/Cargo.toml` | `=0.103.13` | `=0.103.15` | Compatible patched rustls peer |
| `rustls` in both Stage12 auth harnesses | `=0.23.40` | `=0.23.45` | Keep independent security proofs on the fixed version |
| `rustls-webpki` in both Stage12 auth harnesses | `=0.103.13` | `=0.103.15` | Match the fixed version |

The narrow `cargo update -p rustls --precise 0.23.45` resolution changed only
these lockfile packages:

- `rustls` 0.23.40 → 0.23.45
- `rustls-webpki` 0.103.13 → 0.103.15
- `aws-lc-rs` 1.17.0 → 1.18.1
- `aws-lc-sys` 0.41.0 → 0.45.0
- added `pkg-config` 0.3.34, required by the selected `aws-lc-sys`

No unrelated dependency was updated and no rustls 0.24 dependency was added.
The new `Cargo.lock` SHA-256 is
`fbc16d90daaf58ba7f3f32f10615f8521103a5c019f98132ee37c36928458704`.
The prior expected hash `09b18cac90690855e4bee28cabc59ee713f14baf50f2efbb20f2910ed46047b4`
is superseded by this explicitly authorized security update. Previous
Stage19/Stage20 release artifact hashes are obsolete for the eventual RC.

## Permanent malformed-handshake regression

The permanent test is
`crates/werewolfd/src/tls_identity.rs::tests::tls13_plaintext_encrypted_extensions_after_server_hello_are_rejected`.
It generates the Pelt TLS identity in memory, creates a genuine TLS 1.3
ServerHello using rustls, appends a syntactically valid empty plaintext
EncryptedExtensions handshake message to the same TLS record, and passes that
record through the production exact-SPKI client configuration. On rustls
0.23.45 the client rejects it with
`PeerMisbehaved::KeyEpochWithPendingFragment`. The focused test and full Rust
suite pass. The 0.23.40 acceptance and 0.23.45 rejection were both observed
during advisory reproduction. The fixture stores and prints no key material.

## Receiver authentication and both secure transports

The Stage12 TLS and Quinn authentication harnesses were updated to rustls
0.23.45 / rustls-webpki 0.103.15 and rerun successfully.

- Generated Pelt TLS identity and exact SPKI pinning remain required.
- Genuine TLS 1.3 `CertificateVerify` succeeds; invalid signatures and
  unrelated SPKIs fail before application data is accepted.
- Selection binds the handshake verifier to the selected Pack peer. A
  different valid Pack peer is not substituted.
- No CA or hostname trust, TOFU, early-data authorization, or session
  resumption was introduced. TLS remains TLS 1.3 only; early data and
  resumption are disabled.
- TCP exact-SPKI/TLS handshake and v3 OPEN/ACK/session setup/target
  authorization/forwarding/revocation/Silver tests pass.
- Quinn exact-SPKI authenticated connection, exporter equality/separation,
  Stage10 channel binding, OPEN/ACK, target authorization,
  forwarding/revocation/Silver tests pass.
- Stage10 replay and channel-binding characterization, Stage9 target
  authorization, and Stage11C authority/Silver/revocation tests pass in the
  full Rust suite.

Stage16 admission and recovery checks pass: bounded preauth admission,
stalled TLS timeout, malformed TLS recovery, permit release, and legitimate
reconnect after hostile input. The final CI chaos run also completed 64
malformed TCP and 64 malformed QUIC inputs with zero malformed-log growth.
No panic or application forwarding was observed for denied/malformed peers.

## Stage13 valid-session traffic characterization

Representative Stage13 captures compared the prior 30-sample certified
baseline with five patched samples per regular workload on TCP and QUIC. All
valid sessions completed. The dependency patch produces a measurable change
in upstream TLS record grouping on ordinary TCP sessions; it does not change
Werewolf application messages. Median TCP TLS record counts were:

| Workload | Baseline | Patched |
| --- | ---: | ---: |
| Handshake / idle | 11 | 11 |
| Tiny / 1 KiB request | 15 | 13 |
| Interactive | 43 | 27 |
| 64 KiB transfer | 199 | 105 |
| 1 MiB transfer | 3,027 | 1,519 |
| Asymmetric upload | 1,525 | 767 |
| Asymmetric download | 6,059 | 3,034 |

For a 1 MiB transfer, median TCP wire bytes changed by -1.72% client-to-server
and -1.13% server-to-client; the 64 KiB medians changed by -0.54% and -0.46%.
The asymmetric-download client-to-server median changed +17.81% while its
server-to-client median changed -0.99%. In QUIC, the broad valid-session
datagram totals remained similar; five-sample medians included +6.0% in the
asymmetric-download client-to-server direction and +22.4% in the small
asymmetric-upload server-to-client direction. These are recorded as
implementation-level traffic observations, not a traffic-morphology redesign
or a traffic-flow confidentiality claim.

## Lifecycle, provisioning, release, and resource tests

All final archive-facing tests below used binaries extracted from the
reproducible patched archive unless identified as the Stage18 baseline in the
upgrade/downgrade test.

- Stage17 lifecycle harness: PASS, including shutdown/restart, TLS lifecycle,
  missing-Pelt handling, Silver lock, and listener readiness.
- Stage18 CLI provisioning harness: PASS, including fresh two-node Pelt/Pack
  setup, secure forwarding, restart intent, concurrent mutation, Silver
  recovery, revocation, and doctor checks.
- Stage19 release harness: PASS. It verifies the exact archive inventory,
  checksums, staging install, no implicit identity creation, idempotent
  reinstall, checksum-failure safety, unsafe staging path rejection,
  execution outside the checkout, and uninstall preserving the Den.
- Stage19 upgrade/downgrade harness: PASS against the final archive. Stage18
  state was upgraded to the final patched binaries without changing Pelt,
  Pack, targets, Silver, or Fang intent; downgrade to the compatible Stage18
  binaries preserved state and forwarding.
- An additional two-node test installed the final archive into a disposable
  staging root, provisioned the actual Den Pelt/Pack records and Silver lock,
  uninstalled and reinstalled, and restarted both nodes. All 15 persisted
  files remained byte-for-byte identical; the Pelt fingerprint, one Pack
  peer, and active Silver were recovered.
- Stage20 CI chaos on the final archive: PASS in 76.042 seconds. It completed
  20 TCP reconnects, 20 QUIC reconnects, 20 mixed cycles, three peer restarts,
  five SIGKILL recoveries, 64 malformed TCP inputs, 64 malformed QUIC inputs,
  preauth saturation/recovery, Silver and revocation in both transports, and
  one 50 MiB transfer per transport. It verified 64 hashes and
  209,843,972 payload bytes; malformed-log growth was zero and post-load CPU
  ticks over one second were zero.
- Targeted extended soak: PASS (seed `20260918`). It completed 1,000 TCP
  reconnects, 1,000 QUIC reconnects, 1,000 mixed cycles, 100 peer restarts,
  100 SIGKILL recoveries, Silver during TCP and QUIC, Pack revocation during
  TCP and QUIC, three preauth saturation/recovery cycles, 64 malformed
  inputs per transport, and ten 50 MiB transfers per transport. It verified
  3,022 digests across 2,109,832,936 payload bytes, with no progressive
  instability, malformed-log growth, or resource-bound failure. The final
  archive executables have the same SHA-256 values as those soak-tested.
- The full Rust suite includes the production 10,000 short-lived QUIC Fang
  route regression; all 10,000 route echoes complete and leases are released.

## SBOM, audit, licenses, and secrets

`cargo audit` used the local advisory database updated 2026-09-18. It scanned
224 locked dependencies and reported zero vulnerabilities, including no
RUSTSEC-2026-0285; `warnings` was empty. The Stage12 standalone auth lockfiles
also audited without advisories.

`packaging/SBOM.json` and `packaging/THIRD_PARTY_NOTICES` were regenerated
twice deterministically from the final dependency graph. The SBOM covers all
224 locked packages and records the Linux release closure. The notices contain
128 deduplicated shipped license texts. Every registry package has a license
declaration or license-file reference; the generator fails closed if one is
missing. Its MIT fallback is used only when the published archive declares an
MIT option but omits the text. Notice line endings and trailing horizontal
whitespace are normalized so the generated release inventory passes
`git diff --check`.

`STAGE20S_SECRET_MATERIAL_AUDIT = PASS`. The committed TLS regression creates
identity material in memory only. The added TLS/auth changes contain no static
private key or PKCS#8 material, key logging, or TLS debug instrumentation. The
15-file release archive contains no key, log, or packet-capture files and no
raw protected payload, traffic secret, exporter, or session key was added to
tests or artifacts.

## Final reproducible artifacts

Two builds used new, empty, isolated target directories and identical
`SOURCE_DATE_EPOCH=1778563200`, toolchain, platform, lockfile, source, and
release inputs. The binaries, inner and outer checksum manifests, release
manifest, and complete tar archive were byte-for-byte identical. The release
manifest records source commit `fc957a08c5eaa37c9fe6044207edb79fc55f4703`,
production-code commit `48b9f9ad0a37b91645fe7ed939892a4b8fef60dd`, and the new
Cargo.lock hash.

| Artifact | SHA-256 |
| --- | --- |
| `werewolfd` | `ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4` |
| `werewolfctl` | `c9c5e5cf17350b675927efd419e7bb970c3e136ff79977ce8101357e07f74818` |
| Linux x86_64 archive | `d2d17c0bae498811c2303e7e7c91b7cf8c61705425a58e29ed6e6e483f182e93` |
| `Cargo.lock` | `fbc16d90daaf58ba7f3f32f10615f8521103a5c019f98132ee37c36928458704` |
| `packaging/SBOM.json` | `2e6a20aca03ff0d122f82f2bcf24e8fe12ffd5e5e8c8e084daa3c3a5a2437b47` |
| `packaging/THIRD_PARTY_NOTICES` | `94bf3a374d019c2e49de864f0a463356692cd1c6193b104589d94a67bb1b93c5` |

The archive contains the expected 15 distribution files. Stage19 confirmed
normal operation from installed binaries outside a Git checkout. The package
version is `1.0.0-rc.1` for local testing only; no tag or public RC was
created.

## Full gates

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline` | PASS; five existing warnings were reviewed, with no remediation warning |
| `cargo test --locked --offline --workspace` | PASS; 188 tests, 0 failed |
| Python integration unittests | PASS; 64 tests |
| `python3 -B tests/integration/run.py` | PASS; 12/12 acceptance checks, including encrypted TCP 50 MiB SHA-256 integrity |
| Stage17 lifecycle harness | PASS |
| Stage18 CLI provisioning harness | PASS |
| Stage19 release and upgrade/downgrade harnesses | PASS |
| Stage20 CI chaos harness | PASS against final archive binaries |
| `git diff --check` | PASS |

Rust test breakdown: werewolf-core 22; werewolfd 154; integration targets 12;
total 188. Clippy's five existing warnings are the `nodeid.rs`
`needless_range_loop`, dead-code findings in `target_policy.rs` and
`authority.rs`, and the existing `replay_v3_tests.rs` warning. No historical
test was removed, ignored, filtered, or weakened.

## Production change inventory relative to Stage20

There is no unrelated production refactor. `crates/werewolfd/src` changes only
by adding the malformed TLS regression test; the intentional runtime change
comes from rustls rejecting an invalid TLS 1.3 encryption-level transition.

- **Stage21 documentation retained:** `CHANGELOG.md`, `KNOWN_LIMITATIONS.md`,
  `QUICKSTART.md`, `README.md`, `RELEASE_NOTES_1.0.0-rc.1.md`, `SECURITY.md`,
  `docs/INDEX.md`, `docs/V1_CLI_CONTRACT.md`, `docs/V1_CRYPTOGRAPHY.md`,
  `docs/V1_LINUX_SERVICE.md`, `docs/V1_PERSISTENCE_CONTRACT.md`,
  `docs/V1_SECURITY_PROPERTIES.md`, `docs/V1_THREAT_MODEL.md`,
  `docs/V1_WIRE_CONTRACT.md`, and `packaging/INSTALL.md`.
- **Stage21 release machinery retained:** `packaging/SBOM.json`,
  `packaging/THIRD_PARTY_NOTICES`, `packaging/generate-sbom.py`,
  `packaging/make-release.sh`, `packaging/release.env`, and
  `tests/stage19_release/release.py`.
- **Packaging-only fixes:** preserve the interrupted Stage21 target-directory
  build fix and production-code provenance binding in `packaging/make-release.sh`;
  normalize generated notice whitespace in `packaging/generate-sbom.py` and
  its generated notice output.
- **Security/dependency remediation:** `crates/werewolfd/Cargo.toml`,
  `Cargo.lock`, the Stage12 TLS and Quinn harness manifests/locks/output labels,
  and the permanent test in `crates/werewolfd/src/tls_identity.rs`.

Application wire format: `STAGE20S_WEREWOLF_WIRE_FORMAT_CHANGE = NONE`.
Persistence format: `STAGE20S_PERSISTENCE_FORMAT_CHANGE = NONE`.
CLI surface: `STAGE20S_CLI_SURFACE_CHANGE = NONE`.
The TLS implementation intentionally rejects one malformed handshake that
rustls 0.23.40 accepted; this is upstream TLS enforcement, not a Werewolf
protocol redesign.

## Limits and nonclaims

This certification does not claim arbitrary DoS immunity, traffic-flow
confidentiality, complete-state rollback freshness, or certification for
Windows, macOS, ARM, BSD, or containers. Stage20 did not directly execute
netem packet loss/jitter/reordering, direct ENOSPC injection, read-only
filesystem injection, host-clock changes, or Stage20 systemd restart chaos.
Those remain known limitations.

## C1–C26 certification matrix

| Claim | Result | Evidence |
| --- | --- | --- |
| C1. RUSTSEC-2026-0285 reproduced on prior rustls 0.23.40 | CERTIFIED | Stage21 fixture accepted by 0.23.40; rejection reproduced on 0.23.45 |
| C2. Final graph uses rustls >=0.23.45 | CERTIFIED | Direct, Quinn, platform-verifier, and tokio-rustls dependency paths resolve to 0.23.45 |
| C3. Malformed encryption-epoch fixture rejected | CERTIFIED | Permanent production-configuration regression expects `KeyEpochWithPendingFragment` |
| C4. Receiver exact-SPKI authentication intact | CERTIFIED | Stage12 TLS and Quinn auth harnesses; final transport tests |
| C5. TLS 1.3 CertificateVerify intact | CERTIFIED | Correct signature accepted; invalid signature rejected |
| C6. TCP secure transport authenticated and functional | CERTIFIED | TLS/v3 OPEN/ACK, authorization, forwarding, revoke, and Stage18/20 checks |
| C7. QUIC secure transport authenticated and functional | CERTIFIED | Quinn connection/exporter, OPEN/ACK, authorization, forwarding, revoke, and Stage18/20 checks |
| C8. Stage10 replay/channel binding intact | CERTIFIED | Exporter binding/substitution and replay matrices pass |
| C9. Stage9 target authorization intact | CERTIFIED | TCP and QUIC target authorization matrices pass |
| C10. Stage11C authority/Silver/revocation intact | CERTIFIED | Full tests plus Silver/revoke on both transports in chaos and soak |
| C11. Stage16 preauth/resource bounds intact | CERTIFIED | Admission, timeout, malformed recovery, permit release, reconnect tests pass |
| C12. Stage17 lifecycle intact | CERTIFIED | Final-archive lifecycle harness passes |
| C13. Stage18 CLI provisioning intact | CERTIFIED | Final-archive two-node provisioning harness passes |
| C14. Stage19 packaging/install/upgrade behavior intact | CERTIFIED | Final archive, actual Den preservation, upgrade and downgrade pass |
| C15. Stage20 CI chaos passes on patched binaries | CERTIFIED | Final archive chaos result is PASS |
| C16. 10,000-route QUIC regression passes | CERTIFIED | Full Rust suite completes 10,000 echoes and releases route leases |
| C17. Targeted TCP/QUIC soak has no progressive failure | CERTIFIED | 1,000 cycles per transport and 1,000 mixed cycles; resource checks pass |
| C18. 50 MiB TCP and QUIC integrity passes | CERTIFIED | Final archive CI and extended soak hash-verifies transfers on both transports |
| C19. cargo audit no longer reports RUSTSEC-2026-0285 | CERTIFIED | Current audit: 0 vulnerabilities and 0 warnings |
| C20. SBOM/notices match final dependency graph | CERTIFIED | Deterministic regeneration; 224 components; final lock digest embedded |
| C21. Werewolf application wire format unchanged | CERTIFIED | No application protocol code/schema changed |
| C22. Persistence format unchanged | CERTIFIED | No persistence schema or serialization change |
| C23. CLI surface unchanged | CERTIFIED | No CLI command, option, or output contract change |
| C24. Dependency change limited to reviewed remediation | CERTIFIED | Only rustls, rustls-webpki, coupled AWS-LC patch releases, and pkg-config |
| C25. Final artifacts reproducible | CERTIFIED | Two clean isolated builds byte-identical, including full archive |
| C26. No secret material introduced | CERTIFIED | Secret-material audit passes; generated identity remains in-memory only |

## Required result inventory

1. **Remediation starting HEAD:** advisory stop `8a15958`; valid packaging fix
   preserved at `9b3c1de` before forking this branch.
2. **Branch:** `codex/stage20s-rustls-2026-0285`.
3. **Advisory:** RUSTSEC-2026-0285 / GHSA-2mjx-qc3c-rqvc.
4. **Vulnerable resolved rustls:** 0.23.40.
5. **Patched rustls:** 0.23.45.
6. **Dependency paths:** direct `werewolfd`; transitive through Quinn and
   tokio-rustls (both).
7. **Cargo.toml change:** rustls `0.23` → `=0.23.45`; rustls-webpki
   `=0.103.13` → `=0.103.15`; Stage12 harness pins updated to the same pair.
8. **Cargo.lock delta:** rustls .40→.45, webpki .13→.15, aws-lc-rs 1.17.0→1.18.1,
   aws-lc-sys .41.0→.45.0, add pkg-config .3.34.
9. **Cargo.lock SHA-256:** `fbc16d90daaf58ba7f3f32f10615f8521103a5c019f98132ee37c36928458704`.
10. **0.23.40 reproduction:** malformed same-record plaintext EncryptedExtensions
    after ServerHello was accepted.
11. **Final malformed-handshake result:** rejected with
    `KeyEpochWithPendingFragment`.
12. **Permanent regression path:**
    `crates/werewolfd/src/tls_identity.rs` test
    `tls13_plaintext_encrypted_extensions_after_server_hello_are_rejected`.
13. **TCP exact-SPKI:** PASS.
14. **QUIC exact-SPKI:** PASS.
15. **CertificateVerify:** valid TLS 1.3 signature accepted; invalid signature
    rejected.
16. **TCP forwarding:** PASS, including 50 MiB hash-verified transfer.
17. **QUIC forwarding:** PASS, including 50 MiB hash-verified transfer.
18. **Replay/channel binding:** PASS.
19. **Target authorization:** PASS on TCP and QUIC.
20. **Silver:** PASS on both transports.
21. **Pack revocation:** PASS on both transports.
22. **Stage16:** PASS; admission bounds, TLS timeout, malformed recovery,
    permit release, and legitimate reconnect verified.
23. **Stage17:** PASS on final archive binaries.
24. **Stage18:** PASS on final archive binaries.
25. **Stage19:** PASS; release harness and upgrade/downgrade characterization.
26. **Stage20 CI:** PASS on final archive; 76.042 seconds.
27. **Targeted soak:** PASS; TCP 1,000, QUIC 1,000, mixed 1,000, peer restarts
    100, SIGKILL recoveries 100; ten 50 MiB transfers per transport.
28. **10,000 QUIC routes:** PASS in full Rust suite.
29. **50 MiB TCP:** PASS; SHA-256 payload integrity verified.
30. **50 MiB QUIC:** PASS; SHA-256 payload integrity verified.
31. **cargo audit:** 224 locked packages; 0 vulnerabilities; 0 warnings.
32. **Remaining advisories:** none reported by the 2026-09-18 database.
33. **SBOM:** PASS; 224 components, final lock digest, deterministic output.
34. **Notices/license audit:** PASS; 128 deduplicated shipped license texts,
    deterministic and diff-clean.
35. **Final werewolfd SHA-256:**
    `ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4`.
36. **Final werewolfctl SHA-256:**
    `c9c5e5cf17350b675927efd419e7bb970c3e136ff79977ce8101357e07f74818`.
37. **Final archive SHA-256:**
    `d2d17c0bae498811c2303e7e7c91b7cf8c61705425a58e29ed6e6e483f182e93`.
38. **Reproducibility:** PASS; two clean isolated builds identical byte-for-byte.
39. **Wire format:** `STAGE20S_WEREWOLF_WIRE_FORMAT_CHANGE = NONE`.
40. **Persistence format:** `STAGE20S_PERSISTENCE_FORMAT_CHANGE = NONE`.
41. **CLI surface:** `STAGE20S_CLI_SURFACE_CHANGE = NONE`.
42. **Dependency inventory:** exact reviewed lock delta in item 8; no unrelated
    dependency updates.
43. **Secret audit:** `STAGE20S_SECRET_MATERIAL_AUDIT = PASS`.
44. **Rust breakdown:** werewolf-core 22; werewolfd 154; integration targets 12;
    total 188 passed, 0 failed.
45. **Python result:** 64 integration unit tests passed.
46. **Acceptance result:** 12/12 passed.
47. **Stage20S commits:** `9b3c1de`, `bca5ec0`, `86bd02a`, `5befb1c`,
    `5f2e3b3`, `fc957a0`, followed by the final documentation commit.
48. **Final HEAD:** the commit containing this certification document.
49. **Working tree:** clean after the certification commit.
50. **C1–C26:** all CERTIFIED in the matrix above.
51. **Verdict:** `STAGE20S_RUSTLS_2026_0285_REMEDIATION = CERTIFIED`.

**Stop point:** Stage20S is the new certified baseline. Stage21 was not
resumed; no RC tag, merge, push, or publication was made.
