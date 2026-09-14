# Stage 13E — traffic-volume leakage and bounded shaping study

## Motivation and observer

Stage13B removed the deterministic short TLS record caused by split-prefix
writes. Stage13C found padding-only insufficient; Stage13D found segmentation-
only insufficient. Stage13D's production TCP data classified workloads at
78.3% from frame count and 74.7% from **passively visible total TLS wire
bytes**. Stage13E asks how many *additional* non-application bytes would be
needed to reduce that latter signal. This is cost modeling only: no TCP,
QUIC, Stage10 framing, padding, segmentation, TLS, replay, authority,
authorization or fallback behavior changed.

The observer sees connection boundaries, TCP versus UDP, wire bytes in each
direction, combined bytes, packet/record counts and timing. The **primary
classifier in this study uses volume features only**: client-to-server wire
total, server-to-client wire total, their sum, and their ratio. It does not
use timing or packet cadence. Volume padding changes neither the original
application bytes nor their direction; all modeled extra bytes have real
bandwidth and cryptographic-processing cost. This is a within-Werewolf
workload classifier, not a Werewolf-versus-other-protocol detector.

## Dataset, control and frozen model

The [Stage13E model](../tests/stage13_volume_study/README.md) reads the
committed Stage13D 275-connection production encrypted-TCP dataset: 30
samples for each of nine normal workloads and five 50 MiB samples. It
retains only six public/numeric fields per connection in
`control_dataset.csv`. No new capture was needed. The fixed four workload
groups, per-workload 70/30 split, seed 13, `log1p` train-only scaling and
Euclidean nearest-centroid classifier are inherited from Stage13A–D.
Candidate edges, modes, probabilities, features and decision thresholds were
written to `policies.json` **before candidate evaluation**.

Control A reproduced all 275 directional totals exactly, then matched the
certified passive results before alternatives ran: 34.9% held-out majority,
**74.7% total-only**, **53.0% directional-total-only**, and **38.6%
direction-ratio-only**. Full volume features scored 75.9%. Idle/handshake
connections had 849 client-to-server plus 1382 server-to-client wire bytes;
median tiny totals were roughly 5 KiB, interactive roughly 25 KiB. Current
median directional bytes for upload were 1,333,027/2,768; download
2,491/5,316,892; 50 MiB 2,171/66,490,688. Thus upload/download asymmetry
is conspicuous even without frame-count or timing features.

Every candidate sets an **optimistic completed-connection target**. Mode 1
rounds the combined total upward and apportions additions in the existing
direction ratio; Mode 2 rounds the two directional totals independently;
Mode 3 rounds the combined total and chooses a coarse ratio from
`1/256, 1/16, 1/4, 1/2, 3/4, 15/16, 255/256`, adding further bytes if
needed to avoid reducing either original direction. Mode 1 intentionally
preserves asymmetry; Mode 2/3 can cost more and can themselves create
regular directional patterns. `F_constant` and `G_small64k` use their
applicable Modes 1/2 only. All additions are nonnegative.

| Candidate, frozen before scoring | Total target | Implementability |
| --- | --- | --- |
| A current | Actual measured totals | Existing online behavior; zero addition |
| B fixed | Next of 4 KiB, 16 KiB, 64 KiB, 256 KiB, 1 MiB, 4 MiB, 16 MiB, 64 MiB, 128 MiB | Close-time-only target |
| C power two | Next power of two | Close-time-only target |
| D random upper | B minimum/next/next-two buckets with 70/20/10% seeded draws | Close-time-only, simulation RNG only |
| E coarse | Next of 8 KiB, 128 KiB, 2 MiB, 16 MiB, 128 MiB | Close-time-only, conspicuous classes |
| F constant | Largest observed combined total, or each observed directional maximum | Simulation-only; future workload bound unknown |
| G small | If below 64 KiB, target 64 KiB; otherwise unchanged | Close-time-only target |
| H geometric | Starting at 4 KiB, next 1.5× or 2× bucket | Close-time-only target |

Close-time-only describes knowledge of the final byte count, **not an
implementation**: Stage10 currently has no means to emit arbitrary
authenticated dummy volume. An online incremental policy that guarantees a
final connection target without knowing future application length would
need a separately designed cover protocol and timing rule. F assumes a
dataset-derived maximum, so it is explicitly not a realistic online cap.
All candidates are optimistic final-total upper bounds.

## Classifier, cost and privacy floor

All four classifier columns below use the same split and learner. The
machine-readable `classifier_results.json` includes every candidate/mode
and confusion matrix; `summary.json` contains per-workload overlap and
cost percentiles. Values below show representative Mode 1, plus the key
directional upper bound. Percent cost is the median additional wire bytes
divided by that connection's observed wire bytes; dataset totals are sums.

| Policy/mode | Full volume | Total only | Directional totals | Ratio only | Median added cost | Added bytes across dataset |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| A current | 75.9% | 74.7% | 53.0% | 38.6% | 0% | 0 |
| B fixed, total | 78.3% | 78.3% | 78.3% | 38.6% | 153.9% | 483,668,929 |
| C power two, total | 75.9% | 67.5% | 54.2% | 38.6% | 57.8% | 167,621,569 |
| D random upper, total | 72.3% | 71.1% | 66.3% | 38.6% | 213.8% | 1,747,739,585 |
| E coarse, total | 45.8% | 56.6% | 45.8% | 38.6% | 267.2% | 1,190,556,609 |
| G small 64 KiB, total | 75.9% | 67.5% | 75.9% | 38.6% | 153.9% | 8,623,966 |
| H geometric 1.5×, total | 63.9% | 68.7% | 49.4% | 38.6% | 30.2% | 51,618,053 |
| H geometric 1.5×, directional | 67.5% | 67.5% | 56.6% | 34.9% | 35.4% | 78,349,855 |
| F constant combined, Mode 1 | 63.9% | **34.9%** | 63.9% | 38.6% | 257,564% | 17,673,962,914 |
| F constant per direction, Mode 2 | **34.9%** | **34.9%** | **34.9%** | **34.9%** | 262,756% | 18,042,570,714 |

The **theoretical volume-only floor** is the 34.9% majority baseline when
both direction totals are exactly constant. Holding only the combined total
constant leaves ratio/directional leakage: F Mode 1 still scores 63.9% on
full volume features. Mode 2 eliminates volume-feature variation in the
fixture, but forces every connection to 67,856,291 wire bytes (about
64.7 MiB).
At exact final-total histogram resolution, current tiny and 1 KiB workloads
have zero overlap; H 1.5× total raises their overlap to 93.3%, and E coarse
to 100%, while tiny versus interactive still has zero overlap under those
two policies. Current handshake-only and idle are already identical in this
volume view. F directional makes every pair overlap 100% on final volume.
These within-fixture overlaps are recorded for every workload pair in the
machine-readable summary.
This proves mathematical sufficiency *for these volume-only features*, not
practical traffic anonymity.

The **realistic pre-tail floor for all modeled close-time policies is the
unchanged control**: 74.7% total-only, 53.0% directional, 38.6% ratio.
Before any padding tail, the observer has already seen the original burst,
direction changes and application completion. An obvious close-time tail
can itself identify a custom protocol. End-total scores are therefore
optimistic and cannot be interpreted as an online privacy result. Timing,
record counts and traffic volume *during* the connection were not modeled;
they may preserve or strengthen the original classification.

## Budget and Pareto frontier

The budget selector ranks the **frozen** candidates by held-out total-only
accuracy, then full-volume accuracy; this is an arithmetic comparison, not
permission to tune edges or deploy. Median added-volume budgets ≤1%, ≤2%,
≤5%, ≤10% and ≤25% select only **A current** (74.7%). At ≤50%, ≤100% and
≤200%, the raw winner is H 1.5× **directional** (67.5% total-only) at 35.4%
median added volume. Its 7.2-point total-only reduction misses the preset
10-point threshold. H 1.5× total lowers full-volume accuracy by 12.0 points
but costs 30.2% median, above the 25% bar. No bounded candidate meets all
predeclared privacy, cost, online-feasibility and fingerprint conditions.

The raw total-only-accuracy/cost Pareto sequence is A (0%, 74.7%), H 1.5×
total (30.2%, 68.7%), H 1.5× directional (35.4%, 67.5%), E coarse total
(267.2%, 56.6%), then F constant combined (257,564%, 34.9%). E's coarse
ratio mode ties its total mode on these two axes. F directional improves the
**full** feature result beyond F combined but costs still more. None is a
production recommendation: all additions require absent protocol semantics
and the bucketed modes create structural markers.

## Small, bulk and asymmetric amplification

Median added bytes and multiplicative **wire-volume** factor per connection
are shown below. The full per-workload p90/p95/p99/max values and all candidate modes
are in `summary.json`.

| Workload | H 1.5× directional | C power-two total | E coarse total | G small64k total | F constant directional |
| --- | ---: | ---: | ---: | ---: | ---: |
| Tiny | +3,317 / 1.68× | +3,317 / 1.68× | +3,317 / 1.68× | +60,661 / 13.45× | +67,851,416 / 13,929× |
| Interactive | +3,689 / 1.15× | +7,209 / 1.28× | +105,513 / 5.13× | +39,977 / 2.56× | +67,830,732 / 2,655× |
| 1 KiB request/response | +3,057 / 1.60× | +3,057 / 1.60× | +3,057 / 1.60× | +60,401 / 12.76× | +67,851,156 / 13,214× |
| 64 KiB | +42,497 / 1.25× | +94,689 / 1.57× | +1,929,697 / 12.52× | 0 / 1× | +67,688,836 / 405× |
| 1 MiB | +933,446 / 1.35× | +1,540,520 / 1.58× | +14,123,432 / 6.32× | 0 / 1× | +65,202,507 / 25.6× |
| Asymmetric upload | +462,364 / 1.35× | +761,805 / 1.57× | +761,805 / 1.57× | 0 / 1× | +66,520,944 / 50.8× |
| Asymmetric download | +738,358 / 1.14× | +3,069,417 / 1.58× | +11,458,025 / 3.15× | 0 / 1× | +62,537,100 / 12.8× |
| 50 MiB | +2,463,471 / 1.04× | +615,621 / 1.01× | +67,724,485 / 2.02× | 0 / 1× | +1,363,048 / 1.02× |

Application-volume amplification is worse for small sessions: a 64-byte
bidirectional tiny exchange forced to 64 KiB is **1024× its application
bytes**, even though the wire factor is 13.45× because current TLS/Stage10
overhead is already substantial. A 2 KiB bidirectional 1 KiB exchange at
64 KiB is 32× application bytes. Idle has zero application bytes, so an
application-volume ratio is undefined; G adds 63,305 wire bytes to its
2,231-byte control, a 29.4× wire factor. F's median constant-directional
addition is 67,830,476 bytes per connection; its p95/max addition is
67,854,060 bytes. Its observed maximum factor exceeds **30,415×**.

Total-only normalization largely preserves directional asymmetry. Current
median upload is 1,333,027/2,768 wire bytes (up/down); current download is
2,491/5,316,892. Power-of-two total rounding changes those to approximately
2,092,795/4,358 and 3,918/8,384,691: still clearly upload and download.
Power-of-two *directional* rounding reaches 2,097,152/4,096 and
4,096/8,388,608: the same asymmetry survives. Only F directional makes
every session 1,342,563/66,513,728, an unnatural fixed roughly 2% upload
fraction that is itself a recognizable pattern. Mode 3 coarse ratios leave
obvious ratio lattice points, including 1/256 for download-heavy sessions.

## Boundary attacks, fingerprints and resource feasibility

An authenticated peer/target can choose sizes just above bucket edges.
For the **total-only bucket rule**, worst adjacent-edge multiplicative jumps
are about 4× for B, 2× for power-of-two/H2, 1.5× for H1.5, and 16× for E.
D's possible two-upper-bucket draw reaches about **64×** at some edges.
G's minimum observed 2,231-byte connection can be forced to 65,536 bytes,
29.4×; directional G reached 58.8× observed. F's fixed target made the
smallest observed connection over 30,000× larger. Ratio normalization can
raise the observed maximum above the simple total-bucket edge ratio; the
per-mode maxima and exact edges are retained in `amplification.json`.
Fixed-list buckets stop padding beyond their top 128 MiB bucket; geometric
rules have finite multiplicative ratios, though absolute added bytes grow
with input. No candidate offers cost-free or unboundedly safe padding.

Power-of-two totals, coarse fixed totals, geometric ladders and repeated
ratio points are potential **protocol fingerprints** separate from workload
privacy. Randomized D reduces exact mapping but has very high p95 cost and
still occupies a small custom bucket set. Close-time padding adds a
potentially obvious tail. F's constant direction
totals are maximally synthetic. The study does not claim that any candidate
resembles ordinary web traffic or hides TLS fingerprint, timing, packet
count, duration, transport choice, IP/port or global correlation.

A future implementation would spend network capacity, AEAD/TLS CPU, buffers
and potentially longer session authority lifetime for **every extra byte**.
The maximum observed single-connection additions are 2,586,991 bytes for
H1.5 total, 67,852,101 bytes for E coarse total, and 67,854,060 bytes for
F directional; D random upper reached 128,907,945 bytes. On metered/mobile
networks these are
material. Any cover/padding submission after Stage11C revoke or Silver
linearization must stop; bytes already accepted by TLS/kernel cannot be
recalled. Padding must terminate at the Werewolf peer and must never become
protected-target payload.

The current Stage10 TCP stream carries authenticated **application frames**
only. Arbitrary volume cannot be obtained by appending random ciphertext.
A future reviewed design would need authenticated receiver-consumed cover
units, a frame-type extension, a suitable transport-level padding facility,
or another explicit protocol mechanism. This study did not assume rustls
offers arbitrary user-controlled record padding, did not modify TLS, and did
not invent cover-frame semantics. QUIC is wholly separate: Quinn packetization,
congestion accounting, ACK/PTO and anti-amplification rules prevent direct
transfer of these TCP cost conclusions.

## Audit, regression and decision

Retained artifacts contain workload labels, public connection identifiers,
numeric directional totals, derived counts/costs, fixed policy definitions,
classifier confusion matrices and public hashes only. There are no payload
contents, raw captures, private Pelt/PKCS#8/X25519 material, TLS/QUIC traffic
secrets, exporter bytes, session secrets or key logs.
**STAGE13E_SECRET_MATERIAL_AUDIT = PASS.** Rust/Python/acceptance gates
passed at 151 / 64 / 12 of 12; no existing test was removed or ignored.
Cargo.lock SHA-256 remains
`c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17`.

**Verdict: VOLUME_NORMALIZATION_EFFECTIVE_BUT_TOO_EXPENSIVE.** The constant
directional theoretical bound reaches the 34.9% majority floor for
volume-only features, but its median added wire cost exceeds 262,000% and
its pre-tail observer sees the unmodified transfer. No frozen bounded
candidate satisfies the 10-point improvement, ≤25% median overhead,
fingerprint and plausible-online-path rule. Recommended Stage13F is to
**stop automatic volume morphing**, or separately evaluate the value and
cost of opt-in privacy profiles. This does not authorize production cover
traffic, timers, pacing or a Stage10 protocol change.
