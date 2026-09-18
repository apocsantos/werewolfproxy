# Security reporting

WerewolfProxy 1.0.0-rc.1 is a Linux x86_64 release candidate. Please include
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
This record describes the audit performed for this candidate and does not
make a claim about future advisories.
