# Stage 13C — residual TCP frame-size leakage study

## Question, scope and frozen construction

Stage13B removed the deterministic 26-byte outer TLS record caused by a
separate inner-length write. Stage13C asks whether **per-existing-frame**
padding can materially reduce the remaining workload-size signal at bounded
cost. This is a simulation and production-path measurement, with **no
production source, protocol, dependency, QUIC, timer, segmentation, batching,
or cover-traffic change**. A passive observer can still see TCP/TLS record
sizes, directions, counts, timing, volume and the TLS handshake fingerprint.
The study does not claim to make Werewolf traffic undetectable.

`write_encrypted_frame` in
`crates/werewolfd/src/transport/tcp_encrypted.rs` accepts payload length
`0..=2046`. For payload length `L`, production computes:

```text
m = max(L + 2, 768)
k = floor((2048 - m) / 128) + 1
j = OsRng.next_u32() mod k
padded_plaintext = m + 128*j             # <= 2048
ciphertext = padded_plaintext + 16       # ChaCha20-Poly1305 tag
inner_submitted_frame = ciphertext + 4   # BE ciphertext length
outer_TLS_record = inner_submitted_frame + 22
                 = padded_plaintext + 42 # observed locked-stack mapping
```

The two-byte plaintext length and zero padding are authenticated inside the
AEAD ciphertext. The selected set is uniform to negligible modulo bias, but
its starting residue and number of choices depend on `L`. At `L=1400`, it is
exactly `{1402,1530,1658,1786,1914,2042}`. The receiver still validates a
maximum ciphertext length of 4096 before allocation; the writer's actual
maximum is 2064 ciphertext bytes and 2068 submitted inner bytes. The normal
forwarder reads up to 1400 bytes at a time. A final partial read therefore
has a different candidate set, even though the policy is random.

## Dataset and baseline validation

A fresh 275-connection production TCP run used the Stage13A/B transparent
relay, normal sender and receiver daemons, and the same ten workloads: 30
connections each except five 50 MiB downloads. The relay did not terminate
TLS. Existing **local length-only forwarding diagnostics** supplied payload
lengths; the passive relay supplied TLS record lengths. Each connection was
checked for contiguous frame counters, one matching TLS forwarding record
per logged payload, exact current bucket support, complete accounting of
both wire directions and exact application integrity. Only grouped numeric
metadata was retained in
[`baseline_frames.csv`](../tests/stage13_padding_study/baseline_frames.csv)
and per-connection features in
[`baseline_sessions.csv`](../tests/stage13_padding_study/baseline_sessions.csv).
The 351,055 frames collapse to 2,933 grouped rows. Pelt Dens and raw daemon
logs were removed. Source HEAD was
`af0a02dea2f11f157eaf81080acb869e15e11426`; Cargo.lock SHA-256 was
`c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17`.

**Control A validated before candidate evaluation.** Every measured frame
matched the production formula; `padded+16`, `padded+20`, and `padded+42`
matched ciphertext, inner submission and passive TLS record sizes. Replaying
all measured padded sizes reconstructed both-direction wire totals exactly
for all 275 connections. The 345,403 frames carrying 1400-byte payloads
covered all six predicted buckets; their distance from uniform was only
0.00109 total variation, below the prespecified 0.02 limit. The fresh
control's application-size/wire Spearman was 0.9267, versus 0.9274 in the
certified Stage13B run; 50 MiB median wire/application ratio was 1.268 in
both. Fresh full/size-only classifier accuracies were 67.5%/74.7%, versus
67.5%/72.3% in Stage13B. That 2.4-point size-only variation is ordinary
fixture variation, not evidence of a production change.

The model distinguishes nonforwarding records rather than miscalling them
application padding. Each connection had 333 bytes of public TLS handshake,
434 bytes of encrypted TLS handshake, 1416 bytes of existing Stage10
challenge/OPEN/ACK records, and 48 bytes of close records: 2231 fixed bytes.
Record-order classification and lengths were checked on every connection;
no TLS secret or Stage10 payload was used. Candidate byte estimates retain
these fixed bytes to reproduce the Stage13A/B whole-connection classifier.
Separate forwarding-only scores exclude them.

## Prespecified candidates and classifier

The [policy definitions](../tests/stage13_padding_study/policies.json), seed
13, thresholds and decision rule were frozen **before classifier evaluation**.
The payload histogram motivated E's 256-byte threshold: 710 of 351,055
frames were at most 256 bytes, with no frames from 257 through 399 bytes;
345,403 frames were exactly 1400 bytes. Segmentation and frame count remain
identical in every simulation.

| Policy | Padded-plaintext choice, maximum 2048 | Observed distinct forwarding TLS sizes | Size entropy (bits) | Largest-size share |
| --- | --- | ---: | ---: | ---: |
| A current | Replay measured current random bucket | 64 | 2.723 | 16.5% |
| B power two | Smallest of 128, 256, 512, 1024, 2048 | 5 | 0.128 | 98.4% |
| C coarse | Smallest of 768, 1024, 1536, 2048 | 3 | 0.116 | 98.4% |
| D random upper | Coarse minimum / next / maximum with fixed 55/30/15 probabilities, clamped | 4 | 1.094 | 54.1% |
| E small flatten | `L<=256`: 1536 or 2048 equally; otherwise measured current size | 64 | 2.722 | 16.5% |
| F constant max | Always 2048, simulation only | 1 | 0 | 100% |

B's sub-768 small buckets are wire-format feasible but would remove the
current small-frame privacy floor. C's sizes came from the measured 1400-byte
dominance and the current 768/2048 bounds, before classifier review. D/E use
Python's seeded RNG for replay only; a future production randomized policy
would require cryptographic randomness. A uses **observed** production sizes
for an exact control; the production random-choice distribution is checked
independently by the six-bucket validation above.

The classifier is the unchanged Stage13A/B four-class, per-workload 70/30
split, seed-13, `log1p`/train-only scaling and nearest-centroid method.
Full features are direction-specific whole-connection byte totals, public
TLS record count and duration; size-only omits duration. These are within-
fixture workload scores, not a Werewolf-versus-other-traffic detector.

| Policy | Full accuracy | Size-only accuracy | Spearman app size / wire estimate | Spearman app size / frame count |
| --- | ---: | ---: | ---: | ---: |
| A current | 67.5% | 74.7% | 0.9267 | 0.9402 |
| B power two | 78.3% | 78.3% | 0.9986 | 0.9402 |
| C coarse | 68.7% | 67.5% | 0.9654 | 0.9402 |
| D random upper | 67.5% | 68.7% | 0.9577 | 0.9402 |
| E small flatten | 67.5% | 75.9% | 0.9064 | 0.9402 |
| F constant max | 67.5% | 78.3% | 0.9402 | 0.9402 |

The held-out majority baseline was **34.9%**. Candidate C lowers size-only
accuracy by 7.2 percentage points relative to the fresh A control, but
worsens full accuracy and makes **98.4%** of forwarding records one size.
D's size-only change is 6.0 points, with 54.1% one size. Neither reaches the
prespecified 10-point materiality threshold, and both create more obvious
bucket fingerprints. E lowers one rank correlation slightly but does not
improve classifier accuracy. No policy is recommended from a classifier
number alone.

## Count, direction and final-frame leakage

The **total forwarding-frame count alone classified 78.3%** of held-out
connections, with the identical split and learner. Directional frame counts
alone classified 56.6%. Median directional frame counts illustrate the
asymmetry: upload 756 client-to-server / 1 server-to-client; download 1 / 3023;
50 MiB download 1 / 37,792. Directional forwarding-byte-only accuracy was
56.6% for A and 67.5% for B; exact directional results for every candidate
are in the machine-readable classifier artifact. Every policy's frame-count
Spearman remains 0.9402 because none changes segmentation.

There were **430 final forwarding frames**, all partial below 1400 bytes,
only 0.12% of all forwarding frames. Final-frame size alone classified 56.6%.
A prespecified ablation replacing only final-frame padded sizes with 2048
raised, rather than lowered, the size-only score from 74.7% to 78.3%.
Final-frame sizes can reveal individual transfer residues, but they are not
the dominant four-class signal in this fixture.

Constant-max F is the important upper-bound experiment: despite making all
forwarding records one size, its size-only score was **78.3%**, exactly the
total-frame-count-only score and 43.4 points above the majority baseline.
Forwarding-only size accuracy, with handshake bytes excluded, was also 78.3%
for A, C, D, E and F. Thus, within the current segmentation and workloads,
**PADDING-ONLY PRIVACY CEILING REACHED** for this classifier. B's forwarding-
only score was higher, 89.2%, because its smaller small-frame buckets make
class boundaries sharper.

## Cost, privacy frontier and amplification

All values below are **median added whole-connection wire bytes relative to
the fresh Stage13B control**. A negative value is a saving, not a privacy
claim. The application message pattern and number of frames are frozen.

| Policy | Tiny | Interactive | 1 KiB | 64 KiB | 1 MiB | 50 MiB | 50 MiB ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| A | 0 | 0 | 0 | 0 | 0 | 0 | 1.268× |
| B | −2,752 | −19,264 | +1,084 | +31,428 | +478,670 | +11,943,142 | 1.496× |
| C | −1,472 | −10,304 | +60 | −16,700 | −288,046 | −7,263,130 | 1.129× |
| D | −1,152 | −6,336 | +636 | +4,484 | +56,537 | +1,435,110 | 1.296× |
| E | +512 | +5,696 | 0 | 0 | +704 | +256 | 1.268× |
| F | +1,088 | +10,176 | +1,084 | +31,428 | +496,466 | +12,513,894 | 1.507× |

For tiny/1 KiB exchanges there are two one-way messages, and interactive
has sixteen; divide the table values by those counts for per-message added
bytes. For example E adds 256 bytes per tiny one-way message and 356 bytes
per interactive one-way message. No timer is introduced, but extra bytes can
increase serialization latency on constrained links; no WAN latency or CPU
benchmark is claimed. The full per-workload distributions and p50/p90/p95/p99
overheads are in [`summary.json`](../tests/stage13_padding_study/summary.json).
Across all connections, B's additional-wire overhead percentiles were
16.6/19.0/21.8/34.9; C's were −10.7/0/2.4/13.4; D's were
1.5/5.3/13.4/21.8; E's were 0/19.9/26.6/35.7; F's were
18.7/34.9/40.3/49.3 at p50/p90/p95/p99.

The frozen budget rule uses the **largest median added-wire percentage among
the eight nonempty workloads**. At each budget ≤2%, ≤5%, ≤10%, ≤25% and
≤50%, C has the lowest raw size-only classifier score among eligible
candidates: 67.5%, with a maximum workload median overhead of 1.14%.
That table is an arithmetic frontier, **not** a deployment recommendation:
C's 98.4%-single-size marker fails the structural-fingerprint condition.
No candidate satisfies all prespecified production-evaluation conditions at
any budget. B/F are especially expensive on bulk (about +18% median wire);
C's bulk saving is large but its custom bucket lattice is conspicuous.

Every candidate remains bounded by the existing 2048-byte padded plaintext
maximum and 2090-byte outer TLS forwarding record in this locked stack.
Relative to the **smallest current feasible record for the same payload**,
worst-case single-frame amplification is 2.58× for A/D/E/F, 1.96× for B and
1.48× for C. For a one-byte payload, F can send 2090 outer bytes versus the
current minimum 810; B sends 170 by dropping the current privacy floor.
An authenticated peer or target can request tiny chunks repeatedly, so even
bounded per-frame amplification can become material bandwidth/CPU cost.
No candidate changes the current 2068-byte maximum inner submission buffer.
Fixed policies need no additional RNG; D/E need a cryptographic draw in any
future production implementation. The maximum AEAD input size remains 2048;
F may encrypt roughly 2.7× the current minimum for tiny frames. No measured
CPU, allocation-rate or congestion result is claimed.

## Security, secret audit and recommendation

Stage10's four-byte ciphertext length, two-byte authenticated plaintext
length, AEAD, nonce counters, X25519/BLAKE3 KDF, challenge/OPEN/ACK and replay
were not changed. Stage11C per-frame authority and Silver, Stage12 exact-Pelt
TLS receiver authentication and passive semantic opacity, and Stage9 target
authorization are unaffected. Candidate padding would remain authenticated
ciphertext and must keep the existing maximum check; no parser ambiguity or
secret-driven policy is introduced. No QUIC data path or Quinn setting was
modified.

Retained CSV/JSON fields are workload labels, counts, lengths, timings,
classifiers and hashes only. A schema/marker audit found no Pelt seed,
private PKCS#8, TLS/QUIC traffic secret, exporter value, private X25519,
session secret, key log or payload content. **STAGE13C_SECRET_MATERIAL_AUDIT
= PASS.** Production Dens/logs were removed after capture. No manifest or
Cargo.lock change occurred. Full regression gates passed: formatting,
Clippy, **151 Rust tests** (21 core unit, 12 core integration, 118 werewolfd),
**64 Python tests**, **12/12 acceptance**, and diff checks. The existing
Stage12 exact-receiver, replay, authority, authorization, fallback and wire-
opacity tests remained in the normal workspace run.

**Verdict: PADDING_ONLY_INSUFFICIENT.** Constant-max padding leaves the
classifier far above baseline because frame count is unchanged, while the
coarser candidates create conspicuous size lattices or add bulk cost. No
production padding change is justified by these data. **Recommended Stage13D:
frame segmentation morphology study**, beginning as modeling of count and
direction leakage, with no automatic pacing, cover traffic, QUIC shaping or
production segmentation change.
