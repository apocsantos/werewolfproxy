# Security reporting

WerewolfProxy 1.0.0-rc.2 is the current Linux x86_64 release candidate,
frozen by the Stage33 candidate audit. Please include
the version, platform, transport, relevant configuration shape, and a minimal
reproduction. Do not include Pelt private files, session secrets, TLS key
material, or application payloads.

For a private report, use the repository's GitHub Security Advisories
reporting flow from the repository Security tab when private vulnerability
reporting is available. Do not publish an exploitable issue before maintainers
have had a reasonable opportunity to assess it. If private reporting is
unavailable, the repository currently publishes no separate security email
address; use the repository issue tracker for non-sensitive reports and first
ask maintainers there for a private channel before sharing exploit details.

There is no stated bounty program. A report is not a promise of a response
time or coordinated-disclosure schedule.

## RC1 security history

The Stage21 prepublication advisory gate found RUSTSEC-2026-0285 in the
previously resolved rustls 0.23.40. No RC was published with that dependency.
Stage20S separately certified the remediation; the RC1 dependency graph uses
rustls 0.23.45 and rustls-webpki 0.103.15. The technical reproduction,
regression result, audit date and scope are recorded in the
[Stage20S remediation certification](docs/STAGE20S_RUSTLS_2026_0285_REMEDIATION.md).
This record describes the audit performed for that candidate and does not
make a claim about future advisories. The 1.0.0-rc.2 advisory audit is part
of its Stage33 candidate audit, run on the final release inputs.

## rc.2 security history

Before the rc.2 audit, four rustls advisories against 0.23.45
(GHSA-c8m5-c952-6cm4, GHSA-h2v8-394m-p7vv, GHSA-w44r-93j7-qv28,
GHSA-vp88-798v-h67f) were remediated by moving to rustls 0.23.46; see the
[Stage33 rustls remediation](docs/STAGE33_RUSTLS_0_23_46_REMEDIATION.md).
The candidate's advisory gate then reported five quinn-proto advisories
(RUSTSEC-2026-0338 to -0342), rustls-webpki RUSTSEC-2026-0382 and
webpki-root-certs RUSTSEC-2026-0383 (lockfile only); they were remediated
by moving to quinn 0.11.12 / quinn-proto 0.11.18, rustls-webpki 0.103.16
and webpki-root-certs 1.0.10. tokio's unsound-warning RUSTSEC-2026-0386
(`AsyncFd`, not used by anything shipped) is accepted, with tokio held at
1.52.3; see the
[Stage33 QUIC and webpki remediation](docs/STAGE33_QUIC_TLS_DEPS_REMEDIATION.md).
