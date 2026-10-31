# Changelog

## 1.0.0-rc.2

Architecture-complete candidate (Stage33) after Stage22R–32: named
services, Pack-only discovery, one-relay routing, provider failover,
7-of-10 governance with admission, activation and sync, death, retirement
and the Tombstone, automatic governance sync, and the Stage32 robustness
fixes (F1, F4, F5). rustls 0.23.46 remediates four advisories; quinn
0.11.12 / quinn-proto 0.11.18, rustls-webpki 0.103.16 and
webpki-root-certs 1.0.10 remediate seven more; tokio 1.52.3's
RUSTSEC-2026-0386 warning is accepted. See the
[release notes](RELEASE_NOTES_1.0.0-rc.2.md).

## 1.0.0-rc.1

First formal Linux x86_64 release candidate after Stage9–20 security,
operational, packaging and adversarial-soak certification. See
[release notes](RELEASE_NOTES_1.0.0-rc.1.md) and [Stage21 audit](docs/STAGE21_RC1_AUDIT_FREEZE.md).

Before publication, the Stage21 advisory gate detected RUSTSEC-2026-0285 on
rustls 0.23.40. The separately certified Stage20S remediation pins rustls
0.23.45 and rustls-webpki 0.103.15. The fixed release candidate intentionally
rejects the malformed TLS 1.3 encryption-level transition. See the
[Stage20S security record](docs/STAGE20S_RUSTLS_2026_0285_REMEDIATION.md).
