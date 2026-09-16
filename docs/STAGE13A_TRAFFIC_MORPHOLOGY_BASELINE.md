# Stage 13A traffic-morphology baseline

Stage 13A is **characterization only**. It changes no production behavior,
wire protocol, trust path, fallback rule, padding rule, timing rule, or
dependency. Stage 12 S1–S12 remain the security boundary. The question is
which packet, record, direction and timing features remain distinguishable
after Stage 12 removed unnecessary plaintext semantics.

## Threat model and non-goals

| Observer | Capabilities | Limit |
| --- | --- | --- |
| P1 passive on-path | IPs, ports, TCP/UDP, lengths, directions, timestamps, lifetime, public TLS/QUIC handshake metadata | Cannot decrypt TLS or application data |
| P2 passive local-network | P1 observations with potentially finer timing | Same cryptographic limit |
| P3 statistical classifier | Many sessions; can train on sizes, bursts, directions, timing, volume and public handshake features | Generalization depends on network and host diversity |
| P4 active network interferer | Drop, delay, reorder where transport permits, reset or block flows | Cannot break the cryptography by assumption |

An active authenticated endpoint is outside Stage 12's traffic-opacity
protection. This stage does not attempt to make Werewolf undetectable or TCP and
QUIC mutually indistinguishable: TCP versus UDP is already visible at IP
level. It does not hide IP addresses, ports, direction, duration or total
volume. It does not spoof browsers, HTTP/3, Cloudflare, YouTube or any other
third-party protocol.

## Production capture topology and workload definitions

Two normal daemons start with fresh Pelt identities in disposable private
Dens. The sender's selected Pack peer has the receiver's full Pelt public
key, but its address points to a transparent relay. For TCP, the relay
forwards raw bytes to the production encrypted-TCP listener. For QUIC, a UDP
proxy forwards datagrams to the production Quinn listener, using connection
IDs to route late responses to their original client. Neither terminates
TLS, decrypts QUIC, alters payloads, injects application bytes or keeps raw
captures. The receiver authorizes a single local test target. Each sample is
a new production Fang application connection, causing an independent secure
receiver-authenticated connection and normal Stage 10 OPEN/ACK.
The fixture restarts the disposable daemons between QUIC workload batches,
reloading the same persisted Pelts and Pack records, to stay below the
existing 64-per-peer live-connection cap. A validation run reached that cap
after 64 rapid QUIC connections and was excluded. The checkpointed final
sample includes accepted connections across fresh disposable runs; no failed
connection is counted as a morphology sample.

The target and application client verify exact echo/transfer integrity in
memory. Application payload definitions are deterministic; Pelt creation,
TLS/QUIC handshakes and Stage 10 nonces use production randomness. The
disposable Dens are deleted after each run. No keylog, exporter value, Pelt
seed, private PKCS#8 or TLS/QUIC traffic secret enters analysis artifacts.

| Workload | Application behavior | Connections per transport |
| --- | --- | ---: |
| Handshake only | Wait for authenticated target accept, then close without payload | 30 |
| Tiny | 32-byte request and 32-byte echo | 30 |
| Interactive | Eight serial echo exchanges, 16–256 bytes each | 30 |
| 1 KiB | 1 KiB request and echo | 30 |
| 64 KiB | 64 KiB request and echo | 30 |
| 1 MiB | 1 MiB request and echo | 30 |
| 50 MiB bulk | 32-byte request, 50 MiB integrity-checked download | 5 |
| Asymmetric upload | 1 MiB upload, 32-byte receipt | 30 |
| Asymmetric download | 32-byte request, 4 MiB download | 30 |
| Idle | Authenticated target accept, one second without payload, then close | 30 |

There are 275 independent connections per transport, 550 total. The
20-millisecond burst-gap threshold was fixed before collecting results: an
observed unit starts a new burst when it follows the previous unit by more
than 20 ms. Timestamps come from `time.monotonic_ns()`.

For TCP, one captured unit is one relay `recv()` call, **not** an original
TCP segment, sender write, or IP packet. This distinction prevents a false
packetization claim. TLS record lengths and content types are separately
parsed from the complete relayed byte stream without TLS secrets. For QUIC,
one unit is a real UDP datagram; long-header version, connection-ID lengths,
token length and datagram length are parsed publicly. The first 16 observed
lengths retain direction, and the first four per direction appear in the
summary. Per-session outputs also include bidirectional bytes and unit
counts, duration, time from local application send to first response, time
from first transport unit to target accept, median/p90 interarrival gaps,
burst count and first 32 burst sizes, size histogram, largest/smallest/mean/
median unit length, direction changes and up/down byte ratio.

The response latency includes any secure setup still pending when the local
application submits its first byte. The first-unit-to-target-accept time
includes TLS/QUIC, Stage 10 and target connection; it is not a pure TLS
handshake timer. A one-second idle observation cannot rule out longer-period
maintenance traffic. Loopback relay scheduling and `recv()` coalescing are
measurement limits. An additional ten-connection probe authenticates and
decrypts **client** QUIC v1 Initials using only their public salt and
destination connection IDs; it retains only field summaries. Later QUIC
Handshake and 1-RTT packets remain encrypted to this observer, including
unavailable server transport-parameter values.

## TCP/TLS handshake and record behavior

All 275 TCP ClientHellos had a 189-byte handshake message inside a 194-byte
TLS record. The public legacy record/version fields were 0301/0303; the
supported-version extension offered TLS 1.3 (0304), and production still
negotiated TLS 1.3 only. The cipher-suite list was
`1302,1301,1303,c02c,c02b,cca9,c030,c02f,cca8`, supported groups
`29,23,24`, signature scheme `2055` (Ed25519), and one X25519 key share
with a 32-byte public value. SNI and ALPN were absent in 275/275 samples.
The eight extension **types** were identical in all samples, but their order
had 270 variants; byte-for-byte ClientHello ordering is not a stable
fingerprint here. All 275 ServerHellos had a 122-byte handshake message in a
127-byte record, selected suite `1302` and extensions `51,43`.

The full TCP traces contained 705,159 parsed TLS records. Exactly 351,067
(49.8%) were 26-byte application records. A four-byte inner Stage 10 length
prefix written separately becomes 5 TLS header + 4 application + 1 encrypted
content type + 16 tag = **26 bytes**; the immediately following ciphertext
write produces a second, size-varying TLS record. This repeated short/long
pair is the strongest observed avoidable TCP record fingerprint. It reveals
inner frame count and approximate traffic direction even though it does not
expose inner plaintext. The randomized Stage 10 bucket sizes produce broad
record clusters around the permitted padded frame lengths; the outer TLS
layer does not erase those length clusters.

Median TCP observations per independent connection:

| Workload | Wire C→S / S→C bytes | Relay reads C→S / S→C | First-to-last observed ms | First response ms | Wire/app bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| Handshake only | 849 / 1,382 | 4 / 5 | 10.0 | — | — |
| Tiny | 2,513 / 2,982 | 6 / 7 | 111.5 | 111.6 | 84.9× |
| Interactive | 12,689 / 13,414 | 20 / 21 | 774.0 | 111.8 | 18.6× |
| 1 KiB | 2,451 / 2,856 | 6 / 7 | 110.5 | 111.1 | 2.59× |
| 64 KiB | 84,655 / 84,740 | 19 / 21.5 | 122.2 | 122.4 | 1.29× |
| 1 MiB | 1,346,779 / 1,343,526 | 225.5 / 262 | 832.7 | 37.1 | 1.28× |
| 50 MiB bulk | 2,193 / 67,324,032 | 6 / 1,997 | 18,852.6 | 78.9 | 1.284× |
| Asymmetric upload | 1,347,035 / 3,046 | 48 / 6 | 445.2 | 446.5 | 1.288× |
| Asymmetric download | 2,257 / 5,387,366 | 6 / 166.5 | 1,608.5 | 79.3 | 1.285× |
| Idle | 849 / 1,382 | 4 / 5 | 1,017.1 | — | — |

Relay read counts and read-size maxima depend on local socket scheduling;
they are reported for completeness but are excluded from the TCP classifier
and are not claims about Internet packet boundaries. TLS record counts and
lengths are the more defensible passive TCP features.

## QUIC fingerprint and datagram behavior

All 275 measured production QUIC connections used version 1 and emitted a
1,200-byte first client Initial with 20-byte DCID, 8-byte SCID and empty
token; a second 1,200-byte client Initial used 8/8-byte CIDs. True UDP
datagram lengths later clustered at 1,452 and 1,200 bytes in bulk traffic.
The sample first four C→S datagrams for tiny traffic were
`1200,1200,1326,138` bytes; S→C were `1200,138,131,1326`.
Connection-ID **values** and encrypted packet contents vary. The stable
header lengths, version, no-token behavior and 1,200-byte Initial minimum
form a repeatable profile, not proof of uniqueness to Werewolf.

The ten supplemental production Initials all authenticated under publicly
derived v1 Initial keys, and their targets were reached through normal
receiver authentication. Their ClientHello sizes ranged from 238 to 249
bytes; all ten had the same extension **type set** but ten distinct orders.
The set included QUIC transport parameters (extension 57), supported
versions, groups, key share and Ed25519 signature algorithms; SNI and ALPN
were absent in 10/10. Cipher suites were `1302,1301,1303`; the key share
was X25519. Public client transport-parameter values were stable across the
ten samples despite order variation: 30,000 ms max idle timeout, 1,472-byte
max UDP payload, active connection-ID limit 5, 100 initial bidirectional
and unidirectional streams, and 65,535-byte max datagram frame size.
The first Initial contained 892–903 padding bytes to reach 1,200 bytes.
Only the client Initial was decrypted; these public keys do not unlock later
QUIC handshake or application traffic.

Median QUIC observations per independent connection:

| Workload | Wire C→S / S→C bytes | Datagrams C→S / S→C | First-to-last observed ms | First response ms | Wire/app bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| Handshake only | 8,695.5 / 7,585 | 14 / 11 | 47.2 | — | — |
| Tiny | 8,760 / 7,680.5 | 15 / 13 | 53.0 | 32.1 | 256.9× |
| Interactive | 9,777 / 8,667 | 26 / 23 | 64.8 | 38.3 | 13.1× |
| 1 KiB | 9,753 / 8,673 | 15 / 13 | 55.3 | 34.4 | 9.00× |
| 64 KiB | 75,821.5 / 74,745.5 | 64 / 62 | 42.5 | 43.7 | 1.15× |
| 1 MiB | 1,081,722.5 / 1,080,464 | 769 / 764 | 138.6 | 25.8 | 1.03× |
| 50 MiB bulk | 69,731 / 53,631,669 | 1,772 / 36,945 | 2,489.6 | 19.2 | 1.024× |
| Asymmetric upload | 1,080,998.5 / 8,876.5 | 754 / 47 | 134.8 | 110.6 | 1.039× |
| Asymmetric download | 13,523 / 4,296,980.5 | 155 / 2,967 | 224.1 | 23.0 | 1.028× |
| Idle | 8,679 / 7,586 | 13 / 11 | 48.7 | — | — |

## Workload distinguishability, size leakage and timing

Across nonempty workloads, Spearman correlation between application bytes
and total observed wire bytes was **0.924 TCP** and **0.967 QUIC**. TCP public
TLS record count correlated **0.940** with application bytes. QUIC datagram
count correlated **0.919**. Directional byte totals also separate uploads
from downloads. QUIC's largest datagram size was effectively constant across
the sized classes in this fixture; its **count**, rather than its maximum,
carried the bulk-size signal. The 50 MiB median wire/app ratio was 1.284×
for TCP and 1.024× for QUIC. These are full-connection ratios including
handshake and relay observation, not universal protocol overhead constants.

The held-out four-class classifier used 192 training and 83 test connections
**per transport**. Majority-class guessing on the test set is 34.9%.
With byte/record or datagram totals plus timing features, accuracy was
67.5% TCP (balanced 62.5%) and 72.3% QUIC (balanced 68.1%).
The size-only ablation did better: **72.3% TCP** (95% within-fixture Wilson
interval 61.8–80.8%) and **78.3% QUIC** (68.3–85.8%). Both size-only
classifiers recognized all 29 held-out bulk/asymmetric connections; tiny and
medium classes overlapped more. This is evidence of within-transport
workload leakage on this fixture, not a classifier for Werewolf versus other
applications.

Timing carried additional signal but was noisy. Median time from first
transport unit to authenticated target accept was roughly 7–15 ms for TCP
and 12–23 ms for QUIC, depending on workload. Tiny first-response median
was 111.6 ms TCP and 32.1 ms QUIC; eight interactive exchanges produced
first-to-last observed medians of 774.0 and 64.8 ms respectively. Bulk
first-to-last medians were 18.85 s TCP and 2.49 s QUIC. TCP tiny
interarrival p50/p90 were 2.80/42.26 ms; QUIC tiny were 0.47/4.63 ms.
With the prespecified 20 ms gap rule, median TCP burst count was 3 for tiny,
17 for interactive and 80 for 50 MiB; QUIC yielded 2 for each of those
classes on this fast loopback path. TCP relay-read burst boundaries are
observer-dependent, while QUIC datagram bursts are actual UDP observations.
Application exchange sequencing, production inner-frame writes, rustls/
Quinn buffering, relay scheduling, loopback and per-frame diagnostic logging
all contribute. These numbers are **not** WAN latency or clean transport
throughput benchmarks, and this observational study cannot attribute the
timing difference to one cause.

The baseline classifier is intentionally simple. For each transport it
stratifies independent connections by workload into a deterministic 70/30
train/test split (seed 13), standardizes `log1p` features using training data
only, and assigns held-out sessions to the nearest of four prespecified
centroids: idle/empty, tiny/interactive, medium, and bulk/asymmetric. The TCP
classifier excludes relay-read counts and sizes, using direction-specific
byte totals, public TLS record count and duration. The QUIC classifier can
use actual datagram counts, sizes, bursts and direction changes. This is a
same-host loopback baseline, not an Internet-wide detection estimate.

## Existing Stage 10 TCP padding audit

`write_encrypted_frame` reads up to 1,400 application bytes per direction in
normal forwarding and refuses a plaintext frame above 2,046 bytes. It
prepends a two-byte real-length field, sets
`minimum = max(plaintext_length + 2, 768)`, then chooses
`bucket_count = floor((2048 - minimum) / 128) + 1` and
`frame_size = minimum + 128 * (OsRng.next_u32() % bucket_count)`.
Zero bytes fill to that size before ChaCha20-Poly1305 encryption. The inner
wire frame adds a four-byte ciphertext-length prefix and a 16-byte AEAD tag.
For plaintext lengths at most 766 bytes, padded plaintext sizes range from
768 to 2,048 in 128-byte steps. For larger plaintexts, all possible sizes
retain the residue of `plaintext_length + 2` modulo 128 and the padding
choice narrows as the plaintext approaches the cap. Frame count, direction,
timing and total volume remain inferable despite randomized buckets.

The outer rustls layer adds a TLS 1.3 record header, encrypted content-type
byte and tag. Source submits the inner four-byte prefix and ciphertext in
separate writes; the measured 26-byte records confirm that they become
separate public TLS records in this locked stack. The inner
X25519/ChaCha20-Poly1305 protocol is unchanged. Five 50 MiB runs produced
188,965 such prefix records, about 37,793 per connection. Combining the
prefix with its following ciphertext into one outer TLS write would remove
one 22-byte TLS overhead per frame *if* rustls retains that record mapping:
roughly 0.8 MiB per 50 MiB transfer, or about 1.2% of its observed TCP wire
bytes. This is an estimate for a future proof, not a Stage 13A change.

## Existing QUIC padding and idle audit

Werewolf adds no application-layer QUIC padding or cover traffic. Quinn
provides QUIC-required Initial padding and its own transport packetization.
Application streams use the existing copy paths; Stage 13A does not set a
datagram-size target, fragment packets, pace below congestion control or
enable a keepalive. The measured Initial and idle rows distinguish observed
transport behavior from Werewolf-intentional shaping.
The ten public Initial probes found 892–903 bytes of QUIC-required first-
Initial padding; this is transport padding, not Werewolf application padding.

In 30 one-second authenticated idle samples per transport, TCP showed a
median 1,017 ms first-to-last record interval because the eventual close was
visible. QUIC emitted all captured datagrams within 14–68 ms of the start
and showed no periodic datagram during the one-second application idle.
The absence of later datagrams does **not** prove the internal QUIC connection
ended at 68 ms; its logical lifetime can exceed the observer's active-wire
interval. A longer quiet-period or WAN measurement could reveal maintenance
that this bounded test misses.

## Candidate future mechanisms and cost model

These are evaluation candidates, **not** Stage 13A changes. Costs are
qualitative estimates for a later design review, conditional on workload
and network.

| Candidate | Potential gain | Bandwidth / latency / CPU | Complexity, congestion and DoS concern | Mobile/metered cost |
| --- | --- | --- | --- | --- |
| Size buckets | Blur nearby payload lengths | Medium / low / low | Existing inner buckets limit incremental gain; large padding amplifies traffic | Medium |
| Fixed-size application records | Hide per-record size within one class | High for tiny traffic / possible wait / low | Frame-count and volume remain; can create a stronger fixed signature | High |
| Probabilistic padding | Blur deterministic sizes | Configurable / low / low | Distribution tuning and abuse bounds required | Configurable |
| Minimum-size padding | Hide very short messages | Fixed per message / low / low | Weak for bulk and high relative cost for tiny flows | Medium |
| Burst smoothing | Blur application bursts | Low–medium / added delay / medium | Must schedule above Quinn congestion control | Medium radio-on time |
| Bounded jitter | Blur precise timing | None / added tail latency / low | May preserve class signal while harming interactivity | Medium radio-on time |
| Rate-limited pacing | Reduce bulk-rate signatures | None / potentially high bulk delay / medium | Must respect congestion control; may worsen congestion | High radio-on time |
| Optional cover traffic | Obscure idle and volume patterns | High recurring / possible / medium | Battery, metered links, server capacity, DoS amplification, overly regular anomalies | Very high |
| QUIC datagram normalization | Blur packet-size distribution | Medium–high / possible / high | Quinn-owned packetization and congestion behavior constrain safe placement | Medium–high |
| TLS record shaping | Remove avoidable record-size marker while retaining inner crypto | Potentially low or negative / near-zero if no waiting / low | Must preserve Stage 10 framing and test across rustls writes | Low |

Any future shaping belongs at an appropriate application scheduling layer
unless proved otherwise. Do not manually fragment TCP packets or manipulate
timing below Quinn's congestion control. Superficial third-party fingerprint
impersonation is deferred because it can make traffic more anomalous.

## Recommended Stage 13B direction

The highest-value narrow candidate is **TLS record shaping at the existing
encrypted TCP application-write boundary**: investigate submitting each
inner Stage 10 length prefix together with its ciphertext so that rustls
does not emit a recurring 26-byte record. Keep the inner X25519/ChaCha20-
Poly1305 frame format, padding, counters and transcripts exactly as they
are. First prove the write/record mapping and latency in a disposable
prototype, then compare against this baseline over independent connections
and more than loopback before considering production integration. The
observed 50 MiB trace suggests roughly 0.8 MiB **less** TLS record overhead
per transfer if one record replaces each two-record pair. No added waiting
is required by the concept; actual latency/CPU effects remain unmeasured.

QUIC datagram normalization, probabilistic padding and application
scheduling deserve later study only after a bandwidth/latency/DoS budget and
congestion-control review. Cover traffic, generic jitter, fixed-rate pacing
and third-party fingerprint camouflage are deferred: current evidence does
not justify their mobile cost or claim they would improve anonymity.
Stage 13B must preserve Stage 12 S1–S12 and must not remove the inner TCP
AEAD, weaken exact selected-Pelt authentication, or create a plain fallback.

## Stage 12 regression, dependency and secret audit

The source tree characterized was exact Stage 12 certification HEAD
`215f2d453aeed4eaa0700cc4daf743e7d5863f9f` on the new
`codex/stage13-hide-traffic-morphology` branch. The capture intermediate
metadata JSON SHA-256 was
`ac8a0902ce67d233d8cac6037f38703cb030f058995af1fddda28415e47cbf48`.
Its 550 session rows cross-checked against every disposable per-unit CSV:
37,248 TCP relay-read units, 373,693 QUIC datagrams, and 705,159 parsed
TCP TLS records. The committed derived artifacts are
`tests/stage13_traffic_morphology/summary.json`,
`features.csv` and `initial_probe.json`; 53 MiB of detailed per-unit
metadata was held only in a disposable `/tmp` directory for cross-checks
and then removed. It is not committed.
No raw TCP/UDP payload capture is retained by the tool.

`cargo fmt --check`, `cargo clippy --workspace --all-targets`,
`cargo test --locked --offline --workspace`, Python integration unittest
discovery, `python3 -B tests/integration/run.py`, and the staged diff
whitespace check passed. Rust remains **146/146** (21 werewolf-core unit,
12 core integration, 113 werewolfd), Python **64/64**, acceptance **12/12**.
The production source, Stage 9 authorization, Stage 10 crypto/replay,
Stage 11A fallback, Stage 11C authority/Silver and Stage 12 trust/opacity
paths were not edited. Existing Rust and acceptance coverage still exercises
those invariants; no new Stage 12 semantic plaintext leak was observed.

No production Cargo dependency, package, crypto provider or parser changed.
The `Cargo.lock` SHA-256 remains
`c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17`.
The optional public-Initial inspector uses the preinstalled Python
`cryptography` 43.0.0 package only in analysis; it is not a production
dependency. Artifact and source scans found no Pelt private seed, private
PKCS#8, TLS/QUIC endpoint traffic secret, exporter bytes, X25519 private
material, keylog output or hardcoded production secret. Disposable normal
daemon Dens were removed after capture. The inspector derives only the
publicly computable QUIC v1 Initial key in memory and does not retain it.
`STAGE13A_SECRET_MATERIAL_AUDIT = PASS`.

## Environment and claim limits

These captures use Linux loopback, two Pelt identities, one host, one rustls/
Quinn build and a transparent local relay. They do not model WAN congestion,
mobile power, multi-hop jitter, routing changes, different OS packetization,
or a classifier trained on diverse networks. P4 active interference was
threat-modeled but not exercised. Public TLS and QUIC handshake metadata may
remain a stable fingerprint even if application sizes are padded. Stage 13A
does not certify anonymity, traffic-analysis resistance or protocol
impersonation.
