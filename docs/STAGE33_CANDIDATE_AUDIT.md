# Stage33 — 1.0.0-rc.2 candidate audit

**Version:** `1.0.0-rc.2`
**Scope:** Linux x86_64, `x86_64-unknown-linux-gnu`
**Result:** `STAGE33_CANDIDATE_AUDIT = PASS`; tagged `v1.0.0-rc.2` and
frozen (step 5, [below](#step-5-tag-and-freeze))

Step 4 of the [Stage33 survey](STAGE33_CLI_CANDIDATE_SURVEY.md): release
inputs, the archive, the gates of decision 6 and the upgrade test of
decision 7, on the architecture-complete candidate. This record and the
release inputs are documentation, tests and build/package metadata; no
shipped code changed in step 4.

## History

- Survey and decisions: #88 (1a, 2a `1.0.0-rc.2` with 2-i, 3a, 4a, 5a,
  6a, 7a, section 8; 2026-10-29). **Gates changed by the owner to 6b**:
  6a without the WAN smoke.
- Step 2, Cargo package versions `1.0.0-rc.2`: #89. README: #90. Step 3,
  documents: #91.
- Security remediation before the audit:
  - #92, rustls 0.23.46 for four GHSA advisories
    ([record](STAGE33_RUSTLS_0_23_46_REMEDIATION.md));
  - #93, quinn 0.11.12 / quinn-proto 0.11.18, rustls-webpki 0.103.16 and
    webpki-root-certs 1.0.10 for seven RustSec advisories, tokio held at
    1.52.3 ([record](STAGE33_QUIC_TLS_DEPS_REMEDIATION.md)). The first
    step 4 run stopped at the advisory gate and led to #93; its archive
    (`bb7863a7…e43a`) is superseded.
- Release-input commits (rebased onto `main` at `33f7c21`, #93's merge):
  `f0a6db6` (release inputs read from `packaging/release.env`),
  `61bb37c` (release notes, changelog, SBOM and notices), `82dc620`
  (gates 6b), `ebc8d6b` (inputs on the #93 remediation).
- **Archive source commit:** `ebc8d6b6e5724492309eb556c648b3f5e233ce34`.
- **Production-code commit** (last change to shipped code or
  dependencies, recorded in the binaries and manifest):
  `e2ce8992a03bcde7380516b060536351661e35e4`.

## Advisories, dependency graph, SBOM and licenses

`cargo-audit` 0.22.2 (installed for this audit) with the RustSec database
at `2026-10-30T12:12:39+01:00`, database commit
`2295466c4e9df26647fd67d8fa1c705ac2fc7f81`, 1395 advisories, scanned all
218 locked dependencies: **0 vulnerabilities, 1 warning**,
RUSTSEC-2026-0386 (unsound, tokio 1.52.3, `AsyncFd` I/O safety), accepted
by the owner: neither Werewolf nor any shipped dependency other than tokio
uses `AsyncFd` (see the #93 record). This is a dated audit result, not a
claim about future advisories.

- `Cargo.lock` SHA-256
  `a191a20e0e943f726884f5965b93814df4c59145d170de66e3b554ab0b36d27b`, 218
  packages.
- rustls 0.23.46 (`werewolfd` pins `=0.23.46`) is the only rustls; also
  reached through tokio-rustls 0.26.5 and quinn 0.11.12 / quinn-proto
  0.11.18. rustls-webpki 0.103.16 (`=0.103.16`). tokio 1.52.3.
- SBOM (`packaging/SBOM.json`): 218 components, 155 in the Linux release
  closure. `packaging/THIRD_PARTY_NOTICES` covers the shipped closure with
  127 deduplicated license texts.
- `make-release.sh` checks the rustls and rustls-webpki versions in the
  lockfile and the SBOM against `release.env`; the Stage19 harness checks
  the archived SBOM's lockfile hash against the repository's.

## Release inputs and reproducibility

`packaging/release.env`:

```
RELEASE_VERSION=1.0.0-rc.2
RELEASE_TARGET=x86_64-unknown-linux-gnu
PRODUCTION_CODE_COMMIT=e2ce8992a03bcde7380516b060536351661e35e4
RUSTLS_VERSION=0.23.46
RUSTLS_WEBPKI_VERSION=0.103.16
SECURITY_RECORDS="docs/STAGE20S_RUSTLS_2026_0285_REMEDIATION.md docs/STAGE33_RUSTLS_0_23_46_REMEDIATION.md docs/STAGE33_QUIC_TLS_DEPS_REMEDIATION.md"
```

`packaging/make-release.sh` was run twice from `ebc8d6b` with separate
target directories (locked, offline, path prefixes remapped,
`SOURCE_DATE_EPOCH` 1793486870 from the commit time). **The two archives
are byte-identical.**

| Artifact | SHA-256 |
|---|---|
| `werewolfproxy-1.0.0-rc.2-linux-x86_64.tar.gz` | `62d35c135a03f41927f9f70ec8ff2756829bbc4415b4a4993e1a4faa4b1289a3` |
| outer `SHA256SUMS` | `187bde0489538d9c8f7f8646a4ec2343344bfa55ab355e61c3896f4b61521733` |
| `bin/werewolfd` | `737be0091c600a1606afb339553a1a878da0dda8b0296f9904b0946f1b7a33ab` |
| `bin/werewolfctl` | `e752497ef57fee6688c35bcd9a74ac6687c0ab121b1ed8a4203477e7df69a7ed` |
| `systemd/werewolfd.service` | `8b7edb7adbb019b6cca7c51d60820a96a97e67b807649e8c9e9473bd5bff61ea` |
| `RELEASE-MANIFEST.json` | `84f6265bd958cbb5465a354c1929d5a7cad6a78d0c18ea74e6c63ea761afc890` |
| inner `SHA256SUMS` | `dd283d5af5d3ac7eb25a0dc2732c16d77f7080b29af9c27d4a43dbed9152f038` |

The archive holds 18 files: the two binaries, the systemd unit,
`install.sh`, `uninstall.sh`, README, QUICKSTART, INSTALL, SECURITY,
RELEASE_NOTES (`RELEASE_NOTES_1.0.0-rc.2.md`), SBOM, THIRD_PARTY_NOTICES,
RELEASE-MANIFEST.json, RELEASE-METADATA, SHA256SUMS and the three
security records. Inner and outer checksums verify. Both binaries report
`1.0.0-rc.2`; the manifest records `certification_status` "Stage33
1.0.0-rc.2 audit candidate".

## Gates (decision 6b)

Every gate ran on the extracted archive binaries where the gate takes
binaries, and otherwise on the release-input commit `ebc8d6b`, with
disposable daemons only (wolf-a and wolf-b untouched).

| Gate | Result |
|---|---|
| advisories | 0 vulnerabilities; 1 accepted warning (above) |
| `cargo fmt --check`, `cargo clippy --locked --workspace --all-targets -- -D warnings` | pass |
| `cargo test --locked --offline --workspace` | 412 passed, 0 failed |
| Stage19 archive harness (checksums, contents, SBOM and lockfile, install, idempotent reinstall, tampered and unsafe-path refusal, uninstall keeps the Den) | PASS, 7 of 7 |
| Stage17 Linux lifecycle harness | `STAGE17_LIFECYCLE_HARNESS = PASS` |
| Stage18 CLI provisioning harness | `STAGE18_CLI_PROVISIONING_HARNESS = PASS`, 7 of 7 |
| Stage20 CI chaos (`soak.py`, seed 20261029) | PASS: 74.5 s, 64 hashes and 209,843,972 bytes verified, malformed-log growth 0, no fd or thread growth |
| Python unit tests (`tests/integration/test_*.py`) | 64 of 64 |
| acceptance (`tests/integration/run.py`) | PASS, 12 of 12 |
| lab gates | all ten pass: `service_gate` (10), `discovery_gate` (12), `service_discovery_gate` (7), `routed_fang_gate` (12), `provider_gate` (7), `governance_gate` (6), `activation_gate` (4), `sync_gate` (5), `death_gate` (6), `auto_sync_gate` (9) |
| QUICKSTART as written | PASS, 14 checks |
| upgrade from rc.1 (decision 7a) | PASS, 18 checks (below) |

The Stage20 log contains ten `ConnectionResetError` tracebacks from the
harness's own echo threads during its kill and revoke cycles; they are
expected by those cycles and the harness result is PASS.

**QUICKSTART** was followed as written with the archive binaries on
loopback (node addresses `127.0.0.1` with per-node ports, Dens under a
private parent, as QUICKSTART allows): Den, Pelt, Pack and target setup;
an encrypted-TCP Fang and a QUIC Fang with `status` and `doctor`; a named
service; `service publish`, `discover services` and `service add
--from`; and a Fang to a third node through a relay, shown by `fang
active` as "via node-b". Every transfer was a 1 MiB echo with matching
SHA-256.

**Not run (6b):** the WAN smoke of 6a. The Stage22R real-WAN and Stage32
sandbox campaigns are not rerun; their records stay the evidence, tied
to the commits they ran on. The #93 remediation reran a Stage32 QUIC
subset on its own commit (see its record).

## Upgrade from 1.0.0-rc.1 (decision 7a)

The rc.1 binaries were extracted read-only from the frozen archive in
`releases/` (owner permission), which was not otherwise touched: archive
SHA-256 `e2d37273…cea4` and `werewolfd` `ea9ab163…3ac4`, both as recorded
by Stage21; its inner checksums verify. Two nodes on loopback, with the
rc.2 binaries from this archive:

1. **rc.1:** both Dens created, Pelt, Pack, target grant, Silver off and
   an encrypted-TCP Fang; forwarding works.
2. **rc.2 on the rc.1 Dens:** both start; Pelt (fingerprint and key),
   Pack, targets, Fang intent and Silver are unchanged on both nodes; the
   active Fang is restored and forwards; a QUIC Fang created under rc.2
   forwards.
3. **Back to rc.1** on the ungoverned (schema-1) Dens written by rc.2:
   both start and the QUIC Fang created under rc.2 forwards.
4. **Genesis under rc.2:** ten governance keys, a genesis signed by seven
   and combined offline, applied on B with `--expect-hash`; `governance
   status` shows its hash; B's protected manifest is schema 2 with the
   governance document; forwarding to B works.
5. **rc.1 refuses the governed Den** at startup, exit 65,
   `[ERROR][SYSTEM][SECURITY_STATE] invalid protected state generation`.
   **No byte of the Den changed** (a digest of every file, before and
   after).
6. **rc.2 starts again** on that Den with the governance head intact, and
   forwarding to B works.

This matches the documented contract: an rc.1 Den starts under rc.2
unchanged, and a governed Den cannot be rolled back to rc.1 except from a
backup taken before its genesis (INSTALL.md, release notes).

## Out of scope

As decided in section 8 of the survey and listed in
[KNOWN_LIMITATIONS](../KNOWN_LIMITATIONS.md): other platforms and libc;
GUI, mobile, pairing and remote administration; KAD `STORE`, record
relaying, route discovery and more than one relay; automatic provider
reputation; real host clock changes and suspend/resume; the D09
indeterminate commit on real storage; media-level erasure claims; public
release.

## Step 5: tag and freeze

On the owner's word, 2026-11-01, after #94 merged (`fcc0ec7`):

- Annotated tag **`v1.0.0-rc.2`** on `b19215847a6f6b29cf6e017f31507af35677390d`,
  the commit that adds this record (its parent is the archive source
  commit `ebc8d6b`; the commit changes documentation only). The tag is
  pushed.
- The archive above (`62d35c13…89a3`) is the candidate's artifact; it is
  not rebuilt. Its README and SECURITY still carry the pre-freeze
  wording ("in preparation"), as built from `ebc8d6b`; the repository
  copies are updated after the tag.
- `releases/` is unchanged.
