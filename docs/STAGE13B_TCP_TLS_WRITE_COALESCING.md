# Stage 13B — TCP outer-TLS write coalescing

## Hypothesis and scope

Stage13A found that 351,067 of 705,159 public TLS records (49.8%) were
26-byte application records. The suspected cause was the production Stage10
inner encoder writing a four-byte ciphertext length and the ciphertext in
separate `write_all()` calls. Stage13B tests that single cause. It does not
change the Stage10 byte stream, padding, cryptography, TLS trust, authority,
TCP read path, or QUIC.

## Pre-change path and exact change

In `crates/werewolfd/src/transport/tcp_encrypted.rs`,
`secure_copy_client_side` and `secure_copy_server_side` call
`write_encrypted_frame` for each forwarding chunk (the ordinary read buffer
is 1400 bytes). That routine enforces the 2046-byte plaintext maximum,
chooses the existing random padding bucket, prefixes the two-byte plaintext
length, encrypts with the unchanged ChaCha20-Poly1305 key and
direction/counter nonce, then constructs a four-byte big-endian ciphertext
length. Before Stage13B it submitted `write_all(length)` and
`write_all(ciphertext)` separately, followed by `flush()`.

The new routine constructs one bounded `Vec` containing exactly
`length || ciphertext` and calls `write_all(&frame)` once. Its maximum is
4 + 2048 + 16 = **2068 bytes**. The existing per-frame flush remains after
the write; no flush occurred between prefix and ciphertext. There is no
`BufWriter`, explicit yield, timer, new queue, or cross-frame batching in this
path. The two old awaits could yield independently. Outer rustls may still
fragment or coalesce records; one application `write_all()` is the claim, not
one TLS record per frame.

`read_encrypted_frame` remains byte-stream based: `read_exact(4)`, validate
the ciphertext length against 4096 before allocation, `read_exact(length)`,
decrypt, then validate the embedded plaintext length. Shutdown and the
challenge/OPEN/ACK ordering are unchanged. The random bucket formula is
unchanged: `minimum=max(plaintext_len+2,768)`, then one of
`floor((2048-minimum)/128)+1` buckets selected by `OsRng.next_u32()` modulo
that count, spaced by 128 bytes. X25519, BLAKE3 KDF, AEAD keying, nonce
counters, frame limit, and inner padding bytes are untouched.

## Inner-format, failure and authority proof

Five new Rust tests exercise the actual production frame writer. For payload
lengths 0, 1, 16, 32, 256, 1024, 1400 and 2046, the emitted bytes equal the
old conceptual `length.to_be_bytes() || ciphertext`; production
`read_encrypted_frame` recovers the exact payload and checks padding bounds.
An accepting recorder sees one write poll and one unchanged flush per frame.
Four consecutive frames produce four independent writes and parse in order.
A three-byte-at-a-time writer proves `write_all()` completes the same frame
without duplicated prefix. Failures at 0, 2, 4, 16 and 100 accepted bytes
return immediately with no retry; the nonce counter has already advanced once,
as it did before this change. The session's existing error path terminates
the direction rather than resending the frame.

The receiver's `AuthorityTcpStream` remains below rustls and checks the
current Stage11C lease at every low-level write/flush poll. A deterministic
test stops after a partial submission, invalidates the gate, then verifies
that the next poll rejects the remainder. Previously submitted bytes cannot
be recalled; no **new** frame submission is permitted after revoke/Silver
linearization. The existing TCP revoke/Silver and stale-lease tests remain
part of the workspace regression suite. No frame queue or buffering beneath
the authority gate was added.

## Measurement method and provenance

The Stage13A production fixture runs ordinary sender/receiver daemons, the
actual encrypted TCP Fang path, and a real target through a transparent raw
TCP relay. It records relay byte counts and passively parses outer TLS record
headers; it neither terminates TLS nor writes raw payloads to retained
artifacts. The same ten deterministic workload definitions, 20 ms burst rule,
features, and train/test classifier were used. Each dataset has **275 fresh
accepted connections**: 30 per nine ordinary workloads and five 50 MiB
downloads. Payload integrity is asserted in memory.

For an exact short-record comparison, the pre-change reference was rerun from
an archive of Stage13A commit `eb0ac3647e650c6b556ca1a0f3d45e883c247e31`.
Only test capture/analysis options were copied into the disposable archive;
the archived production source remained exact. Its source SHA-256 was
`7789478db67dd8fe3fec966b5ca0183904e77c1d2ab23625bb5d1dfa9aa89280`.
The measured post-change production file SHA-256 was
`c9efad7d2b4e44fd9e0d9e8739d95316e1d77c0c5ae20b9e547ff43cc98fb7c1`.
The reference metadata capture SHA-256 was
`5a39c77a282330108d141cc5cc8af7c8a4a0f4b29fc6f288873b35dda84680d4`;
post-change metadata capture SHA-256 was
`75aff9282f7d58cf88f9c42eaa9d74e21cc79412785e2151f1772dadeaca2686`.
These are metadata-file hashes, not raw-wire-capture hashes. The compact
[comparison](../tests/stage13_tcp_coalescing/comparison.json),
[post-change summary](../tests/stage13_tcp_coalescing/summary.json), and
[features](../tests/stage13_tcp_coalescing/features.csv) contain no payloads.
The original Stage13A aggregate is retained separately.

## Record distribution and size leakage

| Metric | Certified Stage13A | Exact reference rerun | Stage13B |
| --- | ---: | ---: | ---: |
| Independent TCP connections | 275 | 275 | 275 |
| All TLS records | 705,159 | 705,125 | 354,072 |
| Exact 26-byte TLS application records | 351,067 | 351,050 | **0** |
| Exact 26-byte share of all records | 49.8% | 49.8% | **0%** |
| TLS application records ≤32 bytes | not retained exactly | 351,600 | **550** |
| ≤32-byte share of application records | not retained exactly | 49.9% | **0.16%** |
| Median 50 MiB wire/application ratio | 1.284× | 1.285× | **1.268×** |
| Spearman app size vs wire bytes | 0.9237 | 0.9320 | 0.9274 |
| Spearman app size vs TLS record count | 0.9401 | 0.9402 | 0.9402 |

The remaining 550 short records are 24-byte close records, two per
connection. The new most common application-record lengths were 1828,
2084, 1444, 1572, 1700 and 1956 bytes, each about 57–58 thousand records.
They reflect unchanged frame/padding size clusters and are not a new
single-size marker comparable to the old 49.8% 26-byte pattern. Overall
application-size correlation remains high, as expected for a write-only fix.

| Workload | Reference median wire bytes | Stage13B median wire bytes | Reference / Stage13B median TLS records | Reference / Stage13B first response |
| --- | ---: | ---: | ---: | ---: |
| Tiny 32-byte echo | 4,919 | 5,259 | 15 / 13 | 109.9 / 12.1 ms |
| Eight interactive exchanges | 25,527 | 26,263 | 43 / 27 | 110.0 / 13.0 ms |
| 1 KiB echo | 5,243 | 5,135 | 15 / 13 | 111.6 / 12.2 ms |
| 64 KiB echo | 170,227 | 167,647 | 199 / 105 | 118.2 / 104.3 ms |
| 1 MiB echo | 2,688,539 | 2,655,591 | 3,025 / 1,519 | 39.3 / 47.7 ms |
| 50 MiB download | 67,358,993 | 66,472,123 | 75,597 / 37,804 | 98.8 / 19.9 ms |

All five 50 MiB samples sent the defined 32-byte request and received
52,428,800 exact application bytes with SHA-256 equality, no truncation or
deadlock. The post-change median wire total was about 866 KiB lower than the
direct reference. The capture did not make a reliable CPU measurement.
Loopback scheduling, relay behavior and logging preclude an Internet latency
claim. The large tiny/interactive response reduction is consistent with
removing a small first write, but the exact cause is not isolated here.

The unchanged deterministic Stage13A nearest-centroid classifier, with the
same four classes, features, 70/30 stratified split and train-only scaling,
gave full-feature accuracy **67.5% → 67.5%** and size-only accuracy
**74.7% → 72.3%** for the direct reference versus Stage13B. The certified
Stage13A run's corresponding figures were **67.5%** and **72.3%**. These
small differences do not establish workload privacy gain; the strong size
correlations and padding buckets remain.

## Idle, wire-opacity and security regression

The one-second authenticated idle workload generated no periodic application
TLS records; median observed connection duration was 1014.7 ms before and
1014.2 ms after. There is no new keepalive, timer, pacing, or cover traffic.
The existing full-production TCP relay test independently observes actual
Stage10 challenge, OPEN, ACK, sender/receiver fingerprints, target and echoed
sentinel inside production endpoints, then checks that their bytes and the
certificate/SPKI/Pelt public key are absent from the passive raw TLS capture.
It passes with the coalesced writer. Stage12 exact selected-Pelt TLS trust and
CertificateVerify are untouched; QUIC has no production diff.

Full regression gates passed: `cargo fmt --check`,
`cargo clippy --workspace --all-targets`,
`cargo test --locked --offline --workspace`, Python integration unit tests,
12/12 acceptance scenarios, and `git diff --check`. Rust total is **151**
(21 core unit, 12 core integration, 118 werewolfd), five more than the 146
baseline. Python remains **64**. This includes Stage9 target authorization,
Stage10 replay/framing, Stage11A fallback, Stage11B control/Den, Stage11C
revoke/Silver, and Stage12 TLS receiver-authentication/wire-opacity coverage.
Cargo.lock is unchanged at SHA-256
`c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17`.
No dependency was added.

The retained files were checked for Pelt seeds, private PKCS#8, private TLS
or X25519 keys, exporter/traffic/session secrets and keylog material:
**STAGE13B_SECRET_MATERIAL_AUDIT = PASS**. Disposable Pelt Dens were removed;
large raw and per-unit captures were not committed.

## Decision and limits

**STAGE13B_TCP_WRITE_COALESCING = EFFECTIVE.** The dominant deterministic
short TLS-record marker disappeared without changing the inner protocol or
Stage12 security results. One application write does not guarantee one TLS
record on other rustls versions or operating systems. Padding buckets, total
traffic volume, sizes, timing, TCP/TLS fingerprint and traffic correlation
remain observable. These loopback measurements do not characterize WAN
performance or resistance to a trained external classifier.

**Stage13C: GO for a separate, bounded design/measurement stage on residual
frame-size leakage.** Do not infer a need for cover traffic, jitter, QUIC
shaping, or protocol impersonation from this experiment.
