# Changelog

## 1.0.0-rc.1

First formal Linux x86_64 release candidate after Stage9–20 security,
operational, packaging and adversarial-soak certification. See
[release notes](RELEASE_NOTES_1.0.0-rc.1.md) and [Stage21 audit](docs/STAGE21_RC1_AUDIT_FREEZE.md).

Before publication, the Stage21 advisory gate detected RUSTSEC-2026-0285 on
rustls 0.23.40. The separately certified Stage20S remediation pins rustls
0.23.45 and rustls-webpki 0.103.15. The fixed release candidate intentionally
rejects the malformed TLS 1.3 encryption-level transition. See the
[Stage20S security record](docs/STAGE20S_RUSTLS_2026_0285_REMEDIATION.md).
