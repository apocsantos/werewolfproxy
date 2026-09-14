# Stage 13D — TCP frame-segmentation morphology study

## Question and production boundary

Stage13C found that padding all forwarding frames to the legal maximum still
classified workload groups at 78.3% from frame count alone; majority was
34.9%. Stage13D asks whether changing **application-byte segmentation**
would reduce that signal without timers, cover traffic, unbounded buffering
or weakened authority. This is a test-only model. Production TCP, QUIC,
Stage10 padding, crypto, trust, replay, authorization and authority did not
change.

In `crates/werewolfd/src/transport/tcp_encrypted.rs`,
`secure_copy_client_side` (around line 268) reads local application TCP into
a 1400-byte buffer. Each successful `read()` result becomes one
`write_encrypted_frame` call immediately; `0` means EOF and shuts down the
outbound writer. `secure_copy_server_side` (around line 334) does the same
with a 1400-byte target read in the reverse direction. Neither direction
waits for a full read buffer. The read loop waits for `write_all()` and
`flush()` before reading again, so TLS write backpressure can alter later
read boundaries. Tokio readiness and socket availability also influence
partial read sizes. No production `set_nodelay`, `BufWriter`, or other
application buffering layer appears on this forwarding path; ordinary TCP
stack behavior may still affect segment observations.

`write_encrypted_frame` (around line 405) accepts at most 2046 payload
bytes, adds the existing two-byte authenticated plaintext length, selects
current random padding from 768 through 2048, encrypts with
ChaCha20-Poly1305, prefixes four BE ciphertext-length bytes and makes one
outer TLS `write_all()` followed by the preexisting `flush()`. After
decryption, `read_encrypted_frame` reads exactly four bytes, validates the
ciphertext bound, then reads and authenticates that many ciphertext bytes.
Stage13B removed the separate length-prefix TLS write. This study does not
change any of those operations.

The observation boundaries are distinct: **application byte stream →
Stage10 authenticated frames → outer TLS records → TCP packets**. The fresh
production capture found one TLS forwarding record per Stage10 frame in this
locked loopback setup. That is an empirical accounting result, not a promise
that every future frame makes one TLS record or that a TLS record maps to one
TCP packet. Packetization and Nagle behavior were not manipulated.

## Dataset and control

The existing Stage13C grouped file does not preserve exact frame order, so
the [Stage13D capture](../tests/stage13_segmentation_study/README.md) ran a
fresh **275 production encrypted-TCP connections**: 30 independent samples
of each of nine normal workloads and five 50 MiB samples. A transparent raw
relay did not terminate TLS. Existing local length-only diagnostics supplied
the actual per-direction read/frame sequence, and the relay supplied outer
record lengths. The 351,054 frames are retained as ordered, compressed,
metadata-only rows in `frame_dataset.csv.gz`; `sessions.csv` holds 275 public
wire summaries. The dataset was captured from HEAD
`bc2d3d6955fa5125b11802252c7bf90c44c1c76b` with Cargo.lock SHA-256
`c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17`.

**Control A passed before alternatives were evaluated.** Each direction's
frame sequence is contiguous, every observed payload obeys the 1400-byte
read limit, every measured padded size obeys the production formula, and
each record is padded size plus 42 bytes in this capture. Per connection,
the replay reproduces exact application bytes, directional frame counts,
forwarding wire bytes and total bidirectional TLS bytes. Of all frame
payloads, 345,398 are 1400 bytes and 4,596 are 536 bytes; the control has
64 distinct payload lengths. Its 351,054 frames comprise 47,105 client-to-
server and 303,949 server-to-client frames. The control's app-size/frame-
count Spearman is 0.9401, close to Stage13C's 0.9402.

## Fixed candidates and model limits

The [policy file](../tests/stage13_segmentation_study/policies.json) fixed
chunk capacities, randomized probabilities, seed 13 and decision thresholds
before candidate scoring. The passive-wire classifier ablations were added
before final scoring to keep exact application byte totals separate from
actually observable wire totals; no candidate capacity or distribution was
changed. Each candidate preserves byte order and total application bytes.
Changed frames use the **same current Stage10 padding formula**, sampled
with a deterministic simulation RNG; unchanged reads reuse their measured
padding. No policy changes payload contents, frame format, nonce rules,
maximum ciphertext or forwarding protocol.

| Policy | Segment source bytes | Online without waiting? | Distinct payload sizes | Median/p90 payload |
| --- | --- | --- | ---: | ---: |
| A current | One observed read → one frame | Yes | 64 | 1400/1400 |
| B256 | Split each available read at 256 | Yes | 30 | 256/256 |
| B512 | Split each available read at 512 | Yes | 43 | 512/512 |
| C1024 medium | Split each available read at 1024 | Yes | 55 | 536/1024 |
| B1536 or B2046 | Cap exceeds current 1400-byte read | Yes; identical to A | 64 | 1400/1400 |
| D alternate | Capacities 1024,1536 in emitted-frame order | Yes | 56 | 536/1024 |
| E random | Capacities 512/1024/1536/2046 with 10/20/40/30% draws | Yes; CSPRNG needed if ever implemented | 70 | 1400/1400 |
| F available-current-buffer | Maximize only bytes returned by the present read | Yes; identical to A | 64 | 1400/1400 |
| Offline ideal max | Rechunk the **entire known direction** at 2046 | **No**; future-length oracle | 7 | 2046/2046 |

Online B/D/E split a single already-returned read; they do not join bytes
from future reads. F cannot access more than the currently returned 1400
bytes without changing the read architecture or speculatively draining a
socket. Its identical result is a limit of this no-delay model, not proof
that a larger production read would behave identically. The offline ideal
tests the best count reduction possible from whole-direction maximum chunks,
but cannot be implemented online without knowledge of future bytes or a
wait/buffer policy. No timer or delay was simulated. The one-record-per-new-
frame wire estimate uses the observed baseline mapping and is only a
projection; rustls may fragment/coalesce differently after real changes.

## Classifier and leakage results

All scores use the unchanged Stage13A/B/C four workload groups, deterministic
70/30 per-workload split, seed 13, `log1p` train-only standardization and
nearest-centroid classifier. The held-out majority baseline is **34.9%**.
Full uses directional TLS wire totals, TLS record count and measured
duration; size-only omits duration. The model holds observed duration fixed,
so full scores do **not** predict latency changes. Payload-distribution-only
and exact application-byte scores are internal/oracle ablations, not values a
passive observer reads directly from TLS.

| Candidate | Frames | Full | Size-only | Count-only | Direction counts | Payload distribution only | App-size/count Spearman |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| A current | 351,054 | 68.7% | 73.5% | 78.3% | 56.6% | 89.2% | 0.9401 |
| B256 | 2,088,241 | 79.5% | 83.1% | 89.2% | 56.6% | 78.3% | 0.9655 |
| B512 | 1,046,893 | 79.5% | 84.3% | 78.3% | 56.6% | 89.2% | 0.9656 |
| C1024 | 696,591 | 68.7% | 73.5% | 78.3% | 45.8% | 89.2% | 0.9402 |
| B1536/B2046/F | 351,054 | 68.7% | 73.5% | 78.3% | 56.6% | 89.2% | 0.9401 |
| D alternate | 691,858 | 68.7% | 73.5% | 78.3% | 45.8% | 89.2% | 0.9401 |
| E random | 458,563 | 68.7% | 73.5% | 78.3% | 45.8% | 89.2% | 0.9421 |
| Offline ideal | 238,055 | 79.5% | 89.2% | 89.2% | 78.3% | 67.5% | 0.9662 |

Fresh A differs modestly from the previous Stage13C fixture (67.5% full,
74.7% size-only) while reproducing the same qualitative signal. No online
candidate reduces count-only classification from 78.3% by the prespecified
10 percentage points. Smaller chunks either preserve that score or worsen
it, while increasing frame count. C/D/E lower the **directional-count-only**
score to 45.8%, but not total count-only or full scores, and they incur large
byte overhead. The offline ideal's count-only score rises to 89.2%.

Exact application-total-only classification is 89.2%; directional exact
application bytes classify 67.5%, and exact request/response ratio alone
43.4%. These are oracle measures because Stage10 padding hides exact
application byte totals. **Observable** whole-connection TLS wire total
alone classifies 74.7% for current A and 89.2% for the offline ideal;
directional observable wire totals classify 53.0% and 69.9%. Observable
wire up/down ratio alone classifies 38.6% for A and 41.0% for the ideal.
Directional application-size/frame-count Spearman is 0.8956 client-to-server
and 0.9037 server-to-client for A; the offline ideal raises those to 0.9092
and 0.9208. Upload/download asymmetry survives every candidate because
none moves application bytes between directions.
Thus the offline ideal leaves a strong **passively observable volume** signal
even after read-boundary artifacts are removed. It also makes total volume
more predictable by reducing per-frame random padding. This is a measured
loopback-fixture ceiling, not an Internet-wide classifier claim.

## Workloads, overhead and final frames

The table gives median **bidirectional Stage10 frame count** and median
added estimated whole-connection TLS bytes relative to A. Fixed 1536/2046
and F match A exactly. Counts are reported per connection; 50 MiB has five
samples, other rows have 30.

| Workload | A frames | C1024 frames / added bytes | E random frames / added bytes | Offline ideal frames / added bytes |
| --- | ---: | ---: | ---: | ---: |
| Tiny | 2 | 2 / 0 | 2 / 0 | 2 / 0 |
| Interactive | 16 | 16 / 0 | 16 / 0 | 2 / −20,172 |
| 1 KiB | 2 | 2 / 0 | 2 / 0 | 2 / 0 |
| 64 KiB | 94 | 188 / +113,068 | 123 / +35,901 | 66 / −28,244 |
| 1 MiB | 1,508 | 3,001 / +1,794,664 | 1,972.5 / +563,128 | 1,026 / −508,425 |
| Asymmetric upload | 757 | 1,502 / +894,594 | 991 / +280,137 | 514 / −260,646 |
| Asymmetric download | 3,024 | 6,004 / +3,585,200 | 3,954.5 / +1,120,775 | 2,052 / −1,029,240 |
| 50 MiB | 37,793 | 75,030 / +44,835,586 | 49,380 / +13,921,918 | 25,627 / −12,931,990 |

Across all 275 connections the current estimated wire total was
617,909,311 bytes. B256 adds 1,737,187 frames and 2,411,044,704 wire bytes
(+390.2%); B512 adds 695,839 frames and 900,497,592 bytes (+145.7%);
C1024 adds 345,537 frames and 415,621,722 bytes (+67.3%); D adds 340,804
frames and 409,944,560 bytes (+66.3%); E adds 107,509 frames and
129,786,346 bytes (+21.0%). The offline ideal removes 112,999 frames and
projects 120,020,888 fewer bytes (−19.4%), but is not implementable as a
no-delay policy. Each added frame adds a 16-byte AEAD tag, four-byte inner
prefix and, under the empirical mapping, 22 bytes of outer TLS overhead,
**plus another draw of current minimum 768-byte padding**. Exact component
totals, directional counts, workload variance and payload percentiles are in
[`summary.json`](../tests/stage13_segmentation_study/summary.json).

There are 430 terminal forwarding frames (0.12% of current frames). Terminal
payloads below the current 1400-byte read cap are tracked separately in the
machine-readable summary. More numerous split frames make terminal frames an
even smaller fraction; they do not erase total-count or volume leakage. The
simulation does not infer target/application message boundaries from a TLS
record and does not retain application contents.

## Security, resource and fingerprint assessment

Online candidates add **no intentional wait** and keep at most the current
1400-byte read result pending. Their maximum encrypted frame and temporary
submission buffer remain bounded by current Stage10 limits (2048 padded
plaintext, 2068 submitted inner bytes). They submit split frames serially,
so write backpressure must stop further source reads. B256 can multiply a
single 1400-byte read into six frames; B512/E into at most three; C1024/D
into at most two. An authenticated peer or target can force small reads and
repeated padding/AEAD work, but no candidate has unbounded per-read memory.
More writes, encryption and flush opportunities make B256/B512 **high** CPU
and latency risk, C/D **medium to high**, E **medium**, and unchanged caps/F
**low**. These are engineering estimates, not measured WAN latency or CPU.

A future implementation would have to check the Stage11C authority lease
**before each newly submitted frame**, even if a prior read produced several
frames. It could not treat a read buffer as preauthorized after revoke or
Silver. Bounded read → per-frame authority validation → encrypt → submit
preserves the necessary order. Cancellation must stop any unsent remainder;
already submitted bytes cannot be recalled. No background queue, detached
writer, unbounded target drain or new buffer below the authority gate is
allowed. The offline ideal requires future stream length and is explicitly
rejected as a production algorithm.

Fixed 256/512/1024 payload caps would produce conspicuous repeated payload-
length and TLS-size clusters. D's alternating schedule creates a periodic
signature. E reduces exact repetition but introduces a custom four-capacity
lattice and an extra RNG call; production would require a CSPRNG. The
offline 2046-byte runs are also highly regular. Lower within-Werewolf
workload accuracy would not establish camouflage against other protocols.
No candidate is recommended solely from its classifier score.

Stage10 byte ordering, authenticated length/body, padding range, AEAD,
nonces, X25519/BLAKE3 KDF, challenge/OPEN/ACK and replay remain unchanged.
Stage11C authority/Silver, Stage12 receiver-authenticated TLS and wire
opacity, Stage9 authorization and Stage11A fallback are untouched.
Production QUIC is out of scope. No timer, pacing, cover traffic, fake frame,
traffic secret or key log was used.

## Audit, regression and verdict

Retained files contain numeric lengths, counts, direction/workload labels,
classifier summaries and public hashes only. The compressed frame dataset
contains no payload bytes. Disposable production Dens and raw logs were
removed. **STAGE13D_SECRET_MATERIAL_AUDIT = PASS.** Cargo.toml and Cargo.lock
were not changed; the latter remains SHA-256
`c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17`.

Formatting, Clippy, 151 Rust tests (21 core unit, 12 core integration, 118
werewolfd), 64 Python tests, 12/12 acceptance scenarios and diff checks
passed. No previously certified test was removed, ignored or renamed.

**Verdict: SEGMENTATION_ONLY_INSUFFICIENT.** No bounded online no-delay
candidate improves total frame-count classification, and even the offline
whole-stream ideal leaves observable wire-volume and frame-count scores at
89.2%, far above the 34.9% majority baseline. A future Stage13E should
study **traffic-volume leakage and bounded shaping costs**, initially as
analysis. This does not authorize cover traffic, pacing, delay windows or a
production segmentation change.
