# Stage22 private operational validation

## Baseline and artifact

Stage22 starts on `codex/stage22-private-operational-validation` at the exact
Stage21 certification HEAD `27f6c25e2b04a6eeb51e8dc9885b9d28287b35bc`.
The annotated `v1.0.0-rc.1` tag remains unchanged: tag object
`45de84a569d9e9b02598b71f1ad0cb206201456f`, target
`27f6c25e2b04a6eeb51e8dc9885b9d28287b35bc`.

The only operational candidate was the exact Stage21 archive. Its SHA-256 is
`e2d372739743b31359fc115d38f36c3db1c86ebec8afab026b244811c5cacea4`; the
installed `werewolfd` and `werewolfctl` hashes were respectively
`ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4` and
`c9c5e5cf17350b675927efd419e7bb970c3e136ff79977ce8101357e07f74818`.
The outer archive digest and inner `SHA256SUMS` were checked before
installation. The archive installer was exercised with a temporary
`DESTDIR`; tests then invoked the staged archive binaries. No candidate was
rebuilt or relabeled.

Stage21's tag and artifacts were not modified. The Stage22 source change
inventory is limited to `.gitignore`, the harness in `tests/stage22_private/`,
and this report. There is no production source, dependency, wire-format,
persistence-format, or CLI-contract change.

## Harness

`tests/stage22_private/local_campaign.py` pins and verifies the archive,
checks its inner checksums and manifest, stages its own installer, and runs
the Stage17, Stage18, Stage19, and Stage20 CI-safe harnesses against those
staged release binaries. `traffic.py` exercises two local nodes with separate
Dens, generated Pelts, runtimes, and listeners; it covers TCP and QUIC route
churn, transfers, live sessions, target interruption/recovery, concurrent
Fangs, idle-resource bounds, backup/restore, and corruption handling.
`deploy.py` validates an operator-owned mode-0600 node map and can verify and
stage the pinned archive over the operator's existing SSH configuration.
It does not provision identities, start privileged services, or store
credentials. `nodes.example.json` contains documentation-only addresses.

Private node configuration, credentials, Pelt private keys, and Pack secrets
are not present in the repository. The ignored local configuration, private
data, and results paths are listed in `.gitignore`.

## Tested environment and topology

| Environment | Result | Scope |
|---|---|---|
| Debian GNU/Linux 13.6, Linux x86_64, glibc 2.41, kernel 6.12.107 | TESTED | One host; certified installed archive binaries |
| Local nodes A <-> B | PASS | Two isolated Dens, independently generated Pelt identities, separate runtime directories and loopback TCP/QUIC listeners |
| Three-node A <-> B <-> C | NOT TESTED | No third node was available |
| Owner's physical Linux nodes / SSH deployment | NOT TESTED | No private node map or SSH destination was supplied |
| Same LAN, VLAN, routed subnet, VPN, WAN | NOT TESTED | Only loopback on one host was exercised |
| Host reboot / simultaneous host reboot | NOT TESTED | No disposable VM or physical reboot target was available |
| Installed systemd start/enable/boot behavior | NOT TESTED | No privileged host-service test environment was available |
| `tc`/netem impairment | NOT TESTED | `tc` was unavailable |
| Direct ENOSPC, read-only storage, wall-clock step | NOT TESTED | No disposable quota, read-only mount, or time namespace was available |

The tests made no host route, firewall, systemd, disk-policy, or clock
changes. Local outage testing stopped and restored target processes on the
same loopback addresses; it does not represent link, interface, DNS, or WAN
failure.

## Exact-artifact operational results

| Check | Result |
|---|---|
| Archive digest, inner `SHA256SUMS`, embedded manifest and lock digest | PASS |
| Archive installer staged exact daemon and CLI; version commands run outside checkout | PASS |
| Stage17 lifecycle harness on archive binaries | PASS |
| Stage18 provisioning, identity, Pack, target, Fang, TCP, restart, Silver, revocation, status/doctor | PASS |
| Stage19 archive install/uninstall and same-version reinstall/rollback characterization | PASS |
| Stage20 CI-safe chaos on archive binaries | PASS |
| TCP route churn | 10,000 authenticated short-lived routes, PASS |
| QUIC route churn | 10,000 authenticated short-lived routes, PASS |
| Large TCP transfer | 1,024 MiB, end-to-end SHA-256 matched: `c1cafdc97aa4ae8909fdc76b2c7dd970cc30b27117cc24aff1ce9034cf8004f9` |
| Large QUIC transfer | 1,024 MiB, end-to-end SHA-256 matched: `c1cafdc97aa4ae8909fdc76b2c7dd970cc30b27117cc24aff1ce9034cf8004f9` |
| Active TCP / QUIC sessions | 600 seconds per transport with periodic application traffic; both completed |
| Concurrent Fangs | PASS; two simultaneous profiles returned their distinct permitted target markers |
| Target outage and return | PASS for TCP and QUIC; bounded loopback failure and recovery without daemon restart |
| Unattended recovery sequence | PASS; peer stopped, local daemon restarted, peer returned, target returned; fingerprints and Fang intent remained usable without reprovisioning |
| Stopped-Den backup and restore | PASS; private ownership/modes, restored Pelt fingerprint match, daemon startup, and `doctor` passed |
| Corruption drill | PASS; a disposable copy with damaged protected-state selector failed `doctor` closed |
| Test-daemon log scan | PASS; two exercised daemon logs (3,631 bytes total) contained none of the checked private-key, PKCS#8, traffic-secret, session-key, exporter, or known application-payload markers |

The transfer payloads were generated in memory and discarded. Only their
SHA-256 digests were retained. The Pelt fingerprints and private Den contents
were used for equality/mode checks and were not emitted in test output or
this document.

The standalone five-minute idle sample, after provisioning and recovery,
passed all configured bounds. Process RSS changed from 7,824 KiB to 7,952
KiB (+128 KiB), FD count stayed at 16, CPU ticks stayed flat, thread count
changed from 6 to 5, and combined Den size (4,163 bytes) and daemon log size
(3,471 bytes) did not grow.

The exact-artifact Stage20 CI-safe profile also passed in 75.832 seconds. It
verified 64 payload hashes and 209,843,972 application bytes. Relevant
counters included 20 TCP and 20 QUIC reconnects, 20 mixed-transport cycles,
three peer restarts, one simultaneous two-peer daemon restart, five SIGKILL
recoveries, 64 malformed TCP inputs, 64 malformed QUIC datagrams, one TCP
and one QUIC 50 MiB integrity transfer, TCP and QUIC Silver/revocation,
three target-revocation cycles, and one pre-auth saturation/recovery cycle.
Its resource sample was: baseline RSS 8,160 KiB / 22 FDs; peak RSS 18,168
KiB / 23 FDs; post RSS 18,168 KiB / 22 FDs; CPU ticks 3 to 250, then zero
additional ticks over the one-second quiet check. Daemon log size grew from
1,544 bytes to 16,722 bytes during the complete chaos run; malformed-input
log growth was zero. Den size changed from 4,675 to 5,267 bytes as expected
from protected-state operations.

The full-size traffic run completed both transfer equality checks, both
ten-minute session phases, and the five-minute idle bounds. Its first
invocation then stopped in the backup phase because the harness tried to
query Pelt identity with the daemon stopped. The harness was corrected to
start the restored Den and query through its daemon. Backup/restore,
corruption, recovery, and log-audit checks then passed in fresh local runs;
the archive campaign wrapper also passed end to end with a zero-load smoke
profile. The full-size transfer/session profile was not repeated solely to
replay those already completed phases.

The preferred eight-hour session duration was not attempted; each transport
was exercised for ten minutes. The Stage20 CI profile provides additional
shorter soak and resource-quiescence coverage. These observations do not
certify an eight-hour or longer physical-network session.

## Security and source gates

The source baseline has `rustls 0.23.45`, `rustls-webpki 0.103.15`, and
Cargo.lock SHA-256
`fbc16d90daaf58ba7f3f32f10615f8521103a5c019f98132ee37c36928458704`.
`cargo audit` used advisory database timestamp `2026-09-18T11:06:23+02:00`,
database commit `2b34578f89884736e0fcbd42f7ba8d6b10b4a0ce`; it scanned 224
locked dependencies and reported zero vulnerabilities and zero warnings.
`RUSTSEC-2026-0285` remains remediated. The permanent regression
`tls13_plaintext_encrypted_extensions_after_server_hello_are_rejected`
passed as part of the Rust suite.

Exact-SPKI acceptance/rejection, genuine TLS 1.3 CertificateVerify,
replay/channel binding, target authorization, Silver, Pack revocation,
pre-auth admission/resource bounds, and malformed TLS tests passed in the
188-test Rust suite and the installed-artifact Stage17/18/20 harnesses.
Protected-state failure-injection tests passed; no direct ENOSPC or
read-only-filesystem injection was performed.

| Gate | Result |
|---|---|
| `cargo fmt --check` | PASS |
| `cargo clippy --workspace --all-targets` | PASS; five existing warnings (one `needless_range_loop`, four dead-code warnings) |
| `cargo test --locked --offline --workspace` | PASS, 188 total: core 22, daemon 154, integration targets 12 |
| `python3 -B -m unittest discover -s tests/integration -p 'test_*.py'` | PASS, 64 |
| `python3 -B tests/integration/run.py` | PASS, 12/12 |
| Stage17 lifecycle harness | PASS on the exact archive's staged binaries |
| Stage18 provisioning harness | PASS on the exact archive's staged binaries |
| Stage19 release harness | PASS on the exact archive |
| Stage20 CI chaos harness | PASS on the exact archive's staged binaries |
| `git diff --check` | PASS |

## Nonclaims and environment limits

- Linux x86_64 was tested; Windows, macOS, ARM, BSD, and container
  certification are absent.
- No arbitrary DoS immunity or traffic-flow confidentiality is claimed.
- There is no external freshness anchor for complete old Den generations;
  a complete old Den can still be accepted.
- No automatic NAT traversal or public discovery infrastructure is claimed.
- No machine reboot, systemd boot, three-node chain, remote network path,
  `tc`/netem impairment, wall-clock change, direct ENOSPC, or read-only
  storage result is claimed.
- No claim applies to physical networks or machines not tested here.

## C1-C30 certification

| Claim | Result | Evidence / limit |
|---|---|---|
| C1 Exact Stage21 artifact used as operational baseline | CERTIFIED | Pinned outer archive hash, inner sums, manifest, installed binary hashes. |
| C2 Independent node identities preserved | CERTIFIED | Two separate local Dens generated distinct public Pelt fingerprints and keys; restore preserved the original fingerprint. |
| C3 TCP operational forwarding stable | CERTIFIED | 10,000 routes, 1 GiB integrity transfer, ten-minute active session, and loopback target return passed. |
| C4 QUIC operational forwarding stable | CERTIFIED | Same checks as TCP over the authenticated QUIC Fang. |
| C5 Reboot recovery works | NOT TESTED | No disposable VM or physical reboot node. |
| C6 Simultaneous peer restart recovery works | CERTIFIED | Exact-artifact Stage20 CI stopped both daemons, restarted both, and recovered forwarding; host reboot remains untested. |
| C7 Long-lived operation shows no progressive resource failure | CERTIFIED | Ten-minute TCP and QUIC active sessions completed; Stage20 FD count returned from peak 23 to 22 and RSS stabilized at the same 18,168 KiB in immediate, late-load, and post samples. Eight-hour preference not tested. |
| C8 Idle operation shows no spin/log/resource leak | CERTIFIED | Five-minute idle sample: 0 CPU ticks, stable FDs, unchanged Den/log sizes, +128 KiB RSS. |
| C9 High connection churn passes | CERTIFIED | 10,000 TCP and 10,000 QUIC routes through exact installed binaries. |
| C10 Large transfer integrity passes | CERTIFIED | 1,024 MiB per transport; both endpoint SHA-256 values matched. |
| C11 Concurrent Fangs remain correctly isolated | CERTIFIED | Two active profiles returned their own distinct target markers. |
| C12 Silver remains fail-closed operationally | CERTIFIED | Stage18 and Stage20 CI TCP/QUIC Silver checks passed, including active-traffic closure and locked restart state. |
| C13 Pack revocation remains effective | CERTIFIED | Stage18 and Stage20 CI TCP/QUIC revocation checks passed. |
| C14 Target revocation remains effective | CERTIFIED | Stage20 CI completed three target-revocation cycles; Stage18 provisioning also passed. |
| C15 Stage15B persistence survives operational restart | CERTIFIED | Stage17/18/19/20 restart and protected-state regression harnesses passed; active Fang intent recovered. |
| C16 Backup/restore drill passes | CERTIFIED | Complete stopped Den copy retained private modes; restored daemon, fingerprint, and doctor checks passed. |
| C17 Corrupted protected state fails closed | CERTIFIED | Damaged selector on disposable Den copy caused `doctor` to fail closed. |
| C18 Pelt restoration preserves identity | CERTIFIED | Restored Pelt fingerprint matched the pre-backup public fingerprint; no fingerprint disclosed. |
| C19 Storage failure does not falsely publish protected state | CERTIFIED | Protected-state and mutation failure-injection tests passed; direct ENOSPC/read-only tests remain untested. |
| C20 Clock characterization reveals no authentication bypass | NOT TESTED | No safe time namespace or disposable VM was available. |
| C21 Network impairment characterized where available | NOT TESTED | `tc`/netem unavailable; only loopback target-process outages and Stage20 simulated loss were tested. |
| C22 Logs reveal no secret material | CERTIFIED | Exercised daemon logs passed the Stage22 marker scan and Stage21 secret-material review; no secret values were recorded. |
| C23 Systemd lifecycle works operationally | NOT TESTED | Host privileged service install/enable/boot was unavailable. Process lifecycle is covered by Stage17. |
| C24 Unattended recovery characterized | CERTIFIED | Peer outage, local daemon restart, peer return, target outage and return recovered without reprovisioning. |
| C25 Resource observations show no progressive leak | CERTIFIED | Five-minute idle stability and Stage20 CI peak/post resource and quiet checks passed. |
| C26 RUSTSEC-2026-0285 remains remediated | CERTIFIED | `rustls 0.23.45`, zero cargo-audit vulnerabilities/warnings, permanent malformed-epoch test passed. |
| C27 Exact-SPKI/CertificateVerify remain enforced | CERTIFIED | Positive/negative receiver-authentication tests passed in the Rust suite. |
| C28 Stage9/10/11C/16 guarantees remain intact | CERTIFIED | Authorization, replay/channel binding, Silver/revocation, pre-auth and resource regression tests passed. |
| C29 Historical regression suites remain green | CERTIFIED | Rust 188, Python 64, acceptance 12/12, Stage17/18/19/20 all passed. |
| C30 No production behavior changed in Stage22 | CERTIFIED | Diff contains only harness, report, and ignore rules; no production source or dependency changes. |

## Final repository state

Final Stage22 commit and clean-tree state are recorded in the execution
summary. Stage21 tag `v1.0.0-rc.1` still points to
`27f6c25e2b04a6eeb51e8dc9885b9d28287b35bc`. Stage22 did not push, publish,
merge, or alter that tag.
