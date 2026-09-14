# Stage 13 hide / traffic-morphology certification

## Scope, provenance, and threat model

This certifies the Stage 13 source at `eee3b9355430f17015e6189456ec31925ddeef95`, after the Stage 12 certification at `215f2d453aeed4eaa0700cc4daf743e7d5863f9f`. Stage 13A measured production TCP and QUIC; Stage 13B made the sole production change, coalescing one TCP inner frame into one outer-TLS application write. Stages 13C–E modeled alternatives and did not change production padding, segmentation, TCP, or QUIC. This document closes the automatic morphology investigation; it does not certify traffic-flow confidentiality.

A passive on-path observer sees IP addresses, ports, TCP versus UDP, connection boundaries, directions, lengths, timing, total volume, and public TLS/QUIC handshake metadata. A local-network observer may measure timing more precisely. A statistical observer can train across many sessions. An active network interferer can drop, delay, reorder where possible, reset, or block flows, but is not assumed to break cryptography. An authenticated participating endpoint is outside the passive-observer claim and can learn its peer's identity. The measured classifiers distinguish *workloads within Werewolf traffic* on a loopback fixture; they are not Internet-wide classifiers of Werewolf versus other software.

Four properties must remain distinct:

| Property | Current boundary |
| --- | --- |
| Content confidentiality | Secure transports protect application contents from passive decryption. |
| Wire-semantic opacity | Stage 12 keeps internal protocol strings, fingerprints, and requested target out of unnecessary plaintext wire data; receiver authentication precedes sender/target disclosure. |
| Traffic morphology | Stage 13B removed one avoidable TCP TLS-record marker; sizes, counts, directions, timing, and volume remain observable. |
| Traffic-flow confidentiality | **Not claimed.** The present transports do not normalize volume, direction, timing, or session lifetime. |

## Stage 13 evidence and exact production behavior

| Stage | Result | Production effect |
| --- | --- | --- |
| [13A baseline](STAGE13A_TRAFFIC_MORPHOLOGY_BASELINE.md) | `TRAFFIC_MORPHOLOGY_BASELINE = COMPLETE`: 275 fresh connections per transport across ten workloads identified TCP TLS-record and QUIC datagram/Initial fingerprints, workload-size and timing leakage. | None. |
| [13B TCP writes](STAGE13B_TCP_TLS_WRITE_COALESCING.md) | `TCP_WRITE_COALESCING = EFFECTIVE`: the split-prefix record marker collapsed in a direct 275-connection reference comparison. | The only Stage 13 production change. |
| [13C padding](STAGE13C_RESIDUAL_FRAME_SIZE_LEAKAGE.md) | `PADDING_ONLY_INSUFFICIENT`: constant-max per-frame padding still gave 78.3% size-only classification, equal to frame-count-only accuracy; application-size/frame-count Spearman stayed about 0.9402. | None. |
| [13D segmentation](STAGE13D_FRAME_SEGMENTATION_MORPHOLOGY.md) | `SEGMENTATION_ONLY_INSUFFICIENT`: no bounded online/no-wait candidate improved the 78.3% frame-count-only score. Offline whole-stream rechunking retained strong volume leakage. | None. |
| [13E volume](STAGE13E_TRAFFIC_VOLUME_LEAKAGE.md) | `VOLUME_NORMALIZATION_EFFECTIVE_BUT_TOO_EXPENSIVE`: perfect directional-total normalization reached the majority floor for the studied volume features, at extreme modeled cost. | None. |

The sole production diff since the Stage 12 certification is in [`write_encrypted_frame`](../crates/werewolfd/src/transport/tcp_encrypted.rs): it constructs exactly `[4-byte big-endian ciphertext length][existing ciphertext]` and calls one `write_all(&frame)` for each Stage 10 frame. The pre-existing per-frame flush remains. There is no cross-frame queue, timer, pacing, cover traffic, or QUIC production diff. The receiving parser remains a byte-stream parser. One application write is **not** a guarantee of exactly one TLS record on every stack. Five Stage 13B Rust tests verify original inner bytes, padding bounds, one independent write per frame, partial writes, failure/non-retry behavior, and authority invalidation during a partial submission.

The Stage 13A historical capture counted **351,067 / 705,159 (49.8%)** exact 26-byte TLS application records. The direct pre-change rerun counted **351,050**, versus **0** after Stage 13B. Application records of at most 32 bytes fell from **351,600 to 550** in that paired dataset; the 550 remaining were close records. These are fixture counts, not a universal TLS-record-size guarantee. The 50 MiB paired median wire total fell from **67,358,993 to 66,472,123 bytes**, or **1.285× to 1.268×** wire/application ratio. There were fewer TLS records and a strong tiny/interactive first-response improvement on loopback. Neither WAN latency nor CPU savings were established.

Current production therefore retains Stage 12 receiver-authenticated TLS, the Stage 10 TCP inner X25519/ChaCha20-Poly1305 frame format, existing bounded random inner padding, Stage 13B contiguous frame submissions, and ordinary immediate writes without artificial delay. QUIC retains its certified Stage 12 construction. There is no automatic volume normalization, cover traffic, transport impersonation, or TLS/QUIC fingerprint spoofing.

## Residual leakage, privacy cost, and decision

Stage 13C's constant-max simulation flattened every forwarding frame size but left **78.3%** size-only workload classification because frame count was unchanged. Its coarse/power-of-two sizes could become recognizable custom bucket signatures. Stage 13D found no deployable no-wait segmentation candidate that lowered the **78.3%** count-only score; the offline ideal, which requires future knowledge, still left an **89.2%** exact-application-total score and strongly predictive passive wire volume. Padding and segmentation alone cannot conceal the bytes that an application actually transfers.

Stage 13E used the 275-connection production TCP metadata dataset and the same held-out four-class nearest-centroid method. The control scored **74.7% total-wire-only**, **53.0% directional-total-only**, **38.6% ratio-only**, and **75.9% full volume**, versus **34.9% majority**. A theoretical constant target in *both directions* scored **34.9% on all four volume-feature sets** but forced about **67,856,291 wire bytes per connection** in this dataset, with **262,756% median added wire volume** and **18,042,570,714 added bytes** across 275 connections. This is a mathematical bound for completed-connection volume features, not an online privacy result: the original burst, direction changes, completion time, and a conspicuous close-time padding tail remain visible unless a further timing design changes them. No frozen bounded candidate satisfied the preset privacy, median-cost, structural-fingerprint, and plausible-online criteria.

**AUTOMATIC_VOLUME_MORPHING_DEFAULT = REJECTED.** Volume normalization is possible in principle, but the measured automatic-default cost is unacceptable and implementation would require new authenticated protocol semantics and a timing design. The privacy-cost progression is: inexpensive removal of an avoidable write artifact; bounded per-frame padding and segmentation with insufficient improvement; costly total-volume normalization; still costlier directional constant volume; and potentially extreme timing-plus-volume traffic-flow protection. Less workload classification can coexist with *more* protocol recognizability: powers-of-two frames, coarse buckets, fixed connection totals, ratio lattice points, periodic sequences, and padding tails are plausible new markers. Future studies must measure both outcomes separately.

## Current claim boundary and conceptual future profiles

Approved claim: “WerewolfProxy minimizes unnecessary application-semantic leakage on secure transports and removes known avoidable implementation fingerprints, but does not claim full traffic-flow confidentiality.” Traffic volume, timing, direction, and transport metadata remain observable to a passive observer. Claims such as “invisible traffic,” “untraceable traffic,” “undetectable protocol,” “anonymous transport,” or “traffic-analysis proof” are unsupported.

The current default operating point favors authenticated content and wire-semantic opacity with no artificial delay, cover traffic, automatic volume normalization, or protocol impersonation. It is the recommended default. It does not hide source/destination IP or ports, TCP versus QUIC, packet/record lengths, counts, timing, burst structure, duration, total bytes, direction, traffic patterns, or rustls/Quinn fingerprints; it does not resist global correlation or conceal receiver identity from active TLS participants.

An explicit opt-in *bounded privacy* operating point is worth retaining as **paper-only design space**, because some users might accept metered bandwidth and latency cost for reduced workload leakage. It would require separately reviewed bounded volume classes, peer-consumed authenticated cover units, optional directional normalization, an online scheduling rule, and measured amplification/recognizability; no values, names, wire format, or implementation are selected here. A more extreme traffic-flow-confidentiality operating point would likely combine directional volume normalization, timing/pacing, cover traffic, perhaps persistent tunnels, and substantial bandwidth, CPU, battery, and latency cost. It is a different operating point, not a free extension of current Hide. Selection of any future high-cost behavior must be explicit and must never negotiate or imply weaker cryptographic trust.

TCP and QUIC would require separate reviewed mechanisms. QUIC shaping must respect Quinn's congestion control, anti-amplification, ACK/PTO behavior, packetization, flow control, and stream semantics; TCP padding conclusions cannot simply be transplanted. Stage 13 authorizes neither mechanism.

## Future cover, authority, resource, and downgrade rules

Current Stage 10 TCP has authenticated **application** frames only. Appending arbitrary encrypted bytes is not a cover mechanism: a future protocol would need explicit authenticated cover units consumed by the Werewolf peer, never forwarded to the protected target, with reviewed replay/admission behavior. Such units cannot get a weaker authority path than real application data. After Stage 11C revoke/Silver authority linearization, **no new cover or padding transmission may begin under a stale lease**; previously submitted bytes cannot be recalled.

Any later design must bound maximum amplification, cover bytes/session, cover rate, session lifetime, per-peer/global quotas, memory, encryption CPU, and cancellation behavior. Unlimited cover is architecturally excluded. Profile choice must not affect selected-peer Pelt/TLS trust, sender authentication, Stage 9 target authorization, Stage 10 replay, Stage 11A secure fallback, or Stage 11C authority. Morphology policy is independent of security trust and downgrade policy.

## Regression, artifacts, dependencies, and secret audit

The Stage 13 source diff changes only the TCP frame-write submission and adds its five tests. The inner Stage 10 frame, challenge/OPEN/ACK, X25519/KDF/AEAD/nonces, Stage 9 authorization, Stage 10 replay, Stage 11A fallback, Stage 11B local security, Stage 11C authority/Silver, and Stage 12 exact selected-Pelt TLS receiver authentication and wire-semantic opacity remain covered by the full workspace, Python, and acceptance gates. The [Stage 12 certification](STAGE12_HIDE_WIRE_OPACITY_CERTIFICATION.md) remains the narrower semantic-opacity proof. In particular, a Stage 12 full-production raw TCP relay test observes actual challenge/OPEN/ACK at the endpoints and their absence on the passive wire; it still passes after the Stage 13B write change. No Stage 13 analysis commit changed production QUIC.

Stage 13 artifact inventory: A contains test-only TCP/QUIC capture and analysis scripts plus metadata summaries/features; B contains one production writer edit, five Rust tests, comparison tooling, and metadata; C contains test-only frame capture/model code and numeric frame/session datasets; D contains test-only segmentation model code and compressed numeric frame/session metadata; E contains test-only volume model code and numeric totals/candidate outputs. Each stage has a documentation report. Raw traffic captures, payload contents, Pelt/TLS/X25519 private material, traffic/session/exporter secrets, and key logs are not retained in these artifacts. The Stage 13-wide marker and dataset-schema audit found no private-key/key-log markers or payload-content columns. **STAGE13_SECRET_MATERIAL_AUDIT = PASS.** Public hashes, labels, frame lengths, counts, timestamps, and aggregate classifier statistics are retained.

No dependency or provider change occurred during Stage 13. `Cargo.lock` SHA-256 is **`c1ba0b557cb984716c3a04b093df63917cded507fb24ae5a8fbe9f8f04e58d17`**. Final gates passed: `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo test --locked --offline --workspace`, Python integration unit discovery, `tests/integration/run.py`, and diff whitespace checks. Rust: **151 passed** (21 werewolf-core unit, 12 core integration, 118 werewolfd), with zero ignored/filtered; Python: **64 passed**; acceptance: **12/12 passed**. The Stage 13 commit inventory has no deleted or renamed tests, and the five new writer tests remain in normal workspace discovery; no historically certified test was cfg-disabled by Stage 13.

## Individual Stage 13 statements

| Claim | Result | Basis |
| --- | --- | --- |
| M1 — Split-prefix TLS marker removed without changing inner format | **CERTIFIED** | Source diff, five writer tests, 351,050 → 0 direct 26-byte records. |
| M2 — Production TCP preserves Stage 12 wire-semantic opacity | **CERTIFIED** | Full-production passive TCP relay regression and unchanged TLS trust/inner bytes. |
| M3 — Production QUIC behavior unchanged | **CERTIFIED** | Stage 13 production diff is TCP-only; QUIC acceptance remains covered. |
| M4 — Per-frame padding alone insufficient in studied fixtures | **CERTIFIED** | Stage 13C constant-max size-only and count-only both 78.3%. |
| M5 — Segmentation alone insufficient in studied fixtures | **CERTIFIED** | Stage 13D online candidates did not lower 78.3% count-only; offline ideal retained volume signal. |
| M6 — Total/directional volume contributes strongly | **CERTIFIED** | Stage 13E control total 74.7%, directional 53.0%, versus 34.9% majority. |
| M7 — Perfect directional-volume normalization reached volume-feature floor | **CERTIFIED** | Stage 13E theoretical constant-directional result: all volume classifiers 34.9%. |
| M8 — Its automatic-default cost is unacceptable | **CERTIFIED** | About 67.9 million bytes/session; 262,756% median addition; no online protocol. |
| M9 — No arbitrary authenticated cover-frame protocol exists | **CERTIFIED** | Current Stage 10 parser/writer handles authenticated application frames only. |
| M10 — No traffic-flow confidentiality claim | **CERTIFIED** | The explicit observer boundary above retains volume, direction, timing, and transport metadata. |
| M11 — Future morphology profiles cannot weaken security invariants | **CERTIFIED** | Frozen design constraint above; no profile implementation or security-mode coupling. |
| M12 — Automatic volume morphing not recommended by default | **CERTIFIED** | Stage 13E preset decision rule failed; automatic-default decision above. |

**STAGE13_HIDE_TRAFFIC_MORPHOLOGY = CERTIFIED** for the Stage 13 source and evidence named here. This certifies removal of one avoidable TCP marker and an honest characterization of residual morphology leakage, **not** traffic-flow confidentiality or any future profile implementation.
