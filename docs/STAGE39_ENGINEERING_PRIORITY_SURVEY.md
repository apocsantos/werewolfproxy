# Stage39 — post-Stage38 engineering priority survey

**Status: survey for owner review.** It is a read-only audit; no
implementation is authorised. Stage38 stays **CLOSED** (comparative
certification suspended). tokio stays 1.52.3, S3-P2 stays EXCLUDED, and
no historical verdict changes. No production code, `Cargo.lock`,
dependency, harness, timeout, limit, authentication or governance
behaviour was changed to produce it.

**Evidence levels:**

| Level | Meaning |
|---|---|
| CONFIRMED | direct code evidence or a reproducible result |
| SUPPORTED | strong architectural or documented evidence |
| HYPOTHESIS | plausible, needs a test |
| UNKNOWN | insufficient evidence |

Each finding also has a **kind**: defect, operational limitation,
security risk, certification gap, technical debt, or potential feature.

## 1. Executive summary

- **No confirmed security vulnerability.** The authentication and
  authorization path is the same on both transports (section 6).
- **The highest-value bounded problem is relay and target-connection
  liveness.**
  - **The code fact (CONFIRMED):** the receiver's outbound connection to
    its target never gets the kernel liveness settings that every session
    socket gets (keep-alive, `TCP_USER_TIMEOUT` 60 s).
  - **Why it matters:** on a relay, that connection is the WAN hop to the
    destination. A silently dead relay-to-destination path can therefore
    strand routed sessions much longer than any other path allows.
  - **The consequence (HYPOTHESIS):** derived from the code and Linux
    defaults, not yet tested. It sits exactly on a path the project
    records as never exercised.
- **Recommended Stage39: "Relay and target-connection liveness".** It is
  small, security-neutral, uses only existing constants, and is
  testable in the existing sandbox.
- **Four other priorities are ranked** (section 13):
  - QUIC initiator resource-bound parity;
  - root-cause diagnosis of the routed-QUIC regression;
  - CI coverage of the Python, gate and advisory checks;
  - IPv6 transport support and validation.

  Larger directions (multi-relay circuits, metadata privacy, gossip,
  portability) are deferred, with reasons.

## 2. Repository baseline

| Item | Value |
|---|---|
| `main` | `e18fec9c9151be215540132998c8aceb03f7ef11` (#108 merged) |
| Release candidate | `1.0.0-rc.2`, tag `v1.0.0-rc.2` → `b192158`, frozen |
| Toolchain | rustc 1.96.0, cargo 1.96.0 |
| Key dependencies (`Cargo.lock`) | tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18, rustls 0.23.46, rustls-webpki 0.103.16, tokio-rustls 0.26.5, socket2 0.6.3, chacha20poly1305 0.10.1, x25519-dalek 2.0.1, ed25519-dalek 2.2.0 |
| Upstream (crates.io, 2026-11-03) | quinn-proto 0.11.19 is the latest release; tokio 1.54.1 |
| Advisories (`cargo audit --no-fetch`, database `2295466c`, 2026-10-30) | 0 vulnerabilities; 1 warning: RUSTSEC-2026-0386 tokio 1.52.3 (accepted, Stage33). The database was not refreshed for this read-only audit, so refresh it before any release decision. |
| Open PRs and issues | none |
| Workspace | `werewolfd` 27,207 lines, `werewolf-core` 6,232, `werewolfctl` 1,662 (Rust, test code included); 412 Rust test functions |
| CI (`.github/workflows/rust.yml`) | fmt, clippy, `cargo test --workspace --all-features`, `cargo build --release --locked` |

**Certification state:**
- **Stage32:** the network and time campaigns are certified per their
  records.
- **S3-P2:** EXCLUDED (Stage34: the quinn-proto 0.11.18 routed-QUIC
  regression). The `CLOSE-WAIT` stall is kernel backpressure (Stage37,
  Outcome A).
- **Stage38:** CLOSED, `NO_PRACTICAL_VALIDATED_METHOD`
  ([closure](STAGE38_M3B_CLOSURE.md)).
- **Outside certification:** everything in
  [`KNOWN_LIMITATIONS.md`](../KNOWN_LIMITATIONS.md).

## 3. Current architecture and release status

`werewolfd` is one daemon per node. The pieces:

- **Identity:** a Pelt identity (Ed25519), with peers in a Pack
  (exact-SPKI TLS 1.3 binding and signed, connection-bound OPENs).
- **Authorization:** per-target grants (Target Policy), the Silver
  lockout latch, and session authority leases with per-peer and global
  ceilings ([`authority.rs`](../crates/werewolfd/src/authority.rs),
  [`admission.rs`](../crates/werewolfd/src/admission.rs)).
- **Transports:** QUIC and encrypted TCP; plain TCP opt-in only
  ([`TRANSPORT_ARCHITECTURE.md`](../TRANSPORT_ARCHITECTURE.md)).
- **Routing:** one operator-set relay per destination (Stage26), with
  the inner session end to end to the destination.
- **Services and discovery:** Pack-only (Stage23–25), with ordered
  provider failover before establishment (Stage27).
- **Governance and lifecycle:** 7-of-10 statements, admission,
  activation, death and retirement, and automatic sync (Stage28–31).

## 4. Confirmed capabilities (code, tests, records)

Each line separates what is CONFIRMED in the code from what is
certified or exercised, and where.

- **Transport authorization:**
  - **CONFIRMED, code:** both listeners apply the same gates, in the
    same order:
    - the Silver lock check, and a Silver watch for the session's life;
    - an admission permit;
    - the session lease;
    - Target Policy authorization;
    - publication.

    See [`tcp_encrypted.rs:124–257`](../crates/werewolfd/src/transport/tcp_encrypted.rs)
    and [`quic.rs:137–331`](../crates/werewolfd/src/transport/quic.rs).
    QUIC adds a per-connection replay reservation (`quic.rs:322`).
  - **Certified:** Stage12 TLS and Quinn proofs, Stage22R WAN.
- **Memory safety:** no `unsafe` in production code (CONFIRMED). The
  only `unsafe` is a test helper calling `mkfifo` in
  `local_fs/destroy.rs`.
- **Panics:** production panic sites are limited to `std::sync::Mutex`
  poisoning unwraps (`fang_registry.rs`, `handshake.rs`) and signal-handler
  installation at startup (`lifecycle.rs`) (CONFIRMED).
- **Exercised behaviour (documented and certified per their records):**
  restart, crash, path loss, blackhole and storage faults (Stage22R
  WAN); netem impairment, clock jumps and stalls (Stage32 sandbox).

## 5. Known limitations (the existing record, unchanged)

The full list is [`KNOWN_LIMITATIONS.md`](../KNOWN_LIMITATIONS.md). The
ones this survey depends on:

| Limitation | Kind |
|---|---|
| Linux x86_64 only | certification gap |
| IPv6 not run | certification gap; also an implementation limit, see 9.2 |
| Suspend/resume and real clock changes not run | certification gap |
| Indeterminate protected-state commit on real storage unit-tested only | certification gap |
| **Path loss between relay and destination not run** | certification gap; also a code-level concern, see 7.1 |
| Destruction under a full disk not run | certification gap |
| Governance: pull-only delivery, no fork tooling, revoked nodes learn by file | operational limitation |
| No anonymity or traffic-flow confidentiality; one relay hides the requester from the destination only | stated nonclaim |

## 6. Security findings

| # | Finding | Evidence | Kind | Impact |
|---|---|---|---|---|
| S1 | No confirmed authorization bypass or inconsistency between transports | CONFIRMED (code reading, section 4) | — | — |
| S2 | The QUIC **initiator** uses quinn's default `TransportConfig` except keep-alive: no Werewolf bound on its stream and connection receive windows, send window, datagram receive buffer or idle timeout. The receiver sets all of them (`quic.rs:41–62`). | CONFIRMED: [`quic_fang.rs:140–143`](../crates/werewolfd/src/quic_fang.rs) | security risk (resource bound); technical debt | MEDIUM-LOW: only an *authenticated* Pack peer acting as receiver could exploit initiator-side memory (exact-SPKI pinning first). The datagram class is the one RUSTSEC-2026-0342 fixed in quinn-proto 0.11.17+. Extent UNKNOWN until measured. |
| S3 | Stranded relay sessions over a dead relay-to-destination path consume the requester's per-peer quota at the relay (64) and the relay's global quota (1,024) for longer than any other path allows | HYPOTHESIS (see 7.1) | security risk (availability); defect | LOW-MEDIUM: bounded by the quotas, but it lasts until the requester ends the session, then up to the kernel retransmission limit |
| S4 | Advisory state: 0 vulnerabilities, 1 accepted warning, on a database 4 days old | CONFIRMED | certification gap | refresh at the next release gate |
| S5 | No crate-level `#![deny(unsafe_code)]`, although production uses none | CONFIRMED | technical debt (hardening) | LOW |

## 7. Transport findings

### 7.1 Target-connection liveness (the recommended stage)

**Facts (CONFIRMED):**
- `configure_session_socket` (nodelay, keep-alive 30 s + 10 s × 3,
  `TCP_USER_TIMEOUT` 60 s) is applied to:
  - accepted encrypted-TCP sockets ([`tcp_encrypted.rs:124`](../crates/werewolfd/src/transport/tcp_encrypted.rs));
  - outbound session sockets ([`tcp_encrypted.rs:507`](../crates/werewolfd/src/transport/tcp_encrypted.rs)).
- The receiver's connection to its **target**, `TcpStream::connect` via
  `lease.connect_poll`, gets none of them, on either transport:
  - encrypted TCP: [`tcp_encrypted.rs:251`](../crates/werewolfd/src/transport/tcp_encrypted.rs);
  - QUIC: [`quic.rs:338`](../crates/werewolfd/src/transport/quic.rs).
- **On a relay, that target is the destination's encrypted-TCP listener,
  across the WAN** (Stage26: `A ──outer── relay ──TCP── D`).
- **No application idle timeout:** there is none by design; idle
  liveness is the kernel's job (`tcp_encrypted.rs:589`).
- **Host default:** `net.ipv4.tcp_retries2` is 15 on this host
  (Debian), which gives roughly 15 minutes of retransmission before
  unacknowledged data errors.

**Expected consequence (HYPOTHESIS, needs a test).** If the relay-to-D
path blackholes:

- **D's side:** D's accepted socket has keep-alive, so D tears down
  after about 60 s.
- **An idle routed session:** the relay's socket to D has no keep-alive,
  so the relay never notices. The requester's outer session is alive,
  because the relay's kernel answers keep-alives, and the inner session
  has no idle timeout. The routed session therefore stays open with no
  destination behind it, until the requester's application ends it.
- **A session with data in flight:** the relay's unacknowledged writes
  to D are bounded only by `tcp_retries2` (about 15 minutes), not by the
  60 s `TCP_USER_TIMEOUT` that bounds every other Werewolf hop.
- **Direct sessions:** the same applies to a receiver's sessions to any
  remote target host, though usual targets are local.

**Kind:** a defect, if confirmed (a liveness-bound inconsistency), and
a certification gap (the path is recorded as never exercised).

### 7.2 Routed-QUIC regression (A1)

| Question | Answer | Evidence |
|---|---|---|
| Established by bisection | quinn-proto 0.11.15–0.11.17 complete seed 2030's routed QUIC hop under P2 in 4–8 s; 0.11.18 (with quinn 0.11.9 or 0.11.12) and 0.11.19 crawl or fail at setup in about half the runs. Independent of the quinn crate. | CONFIRMED: [Stage34 record](STAGE34_S3_P2_RECORD.md), bisection table |
| Versions tested | quinn-proto 0.11.15, .16, .17, .18 and .19; quinn 0.11.9 and 0.11.12 | CONFIRMED |
| Conditions | S3 routed, QUIC hop A–Y, P2 (10% loss on A's and Y's links), 512 KiB, the cell's own sequence (fresh sandboxes do not reproduce it), seed 2030 | CONFIRMED |
| Cause | Not proven. The leading hypothesis is 0.11.18's send-buffer accounting change (`SendBuffer::buffered` retaining acknowledged data behind gaps). A 4 MiB send-window diagnostic removed the crawl but not the setup resets. | SUPPORTED, not proven |
| Transport, integration or configuration? | The regression is in the dependency (bisection). Integration interacts with it: S2 shows the initiator side runs quinn defaults while the receiver is bounded, an asymmetry nobody has evaluated against the regression. | SUPPORTED |
| Remediation that preserves the bounds | **(a)** wait for a quinn-proto release later than 0.11.19, then the Stage34 retest (no newer release on 2026-11-03); **(b)** a disposable-build diagnosis with quinn tracing or qlog to name the mechanism; **(c)** a minimal upstream report with the Stage34 reproduction (none filed yet); **(d)** any window or timeout change needs architectural review and must not be made to pass S3-P2 | — |
| Future targeted retest | Stage34 "Requirements for future retesting": the seed-2030 cell regression check (at least 6 rounds against the 0.11.17 row), then 30 runs with no FAIL and the kernel-tail count stated | CONFIRMED |

### 7.3 Encrypted-TCP limitations (A2)

Stage37 Outcome A stands: the `CLOSE-WAIT` stall is correct application
behaviour under kernel backpressure and retransmission loss, ended by
`TCP_USER_TIMEOUT`. This is not a Tokio defect.

- **Kinds:** the stall itself is expected TCP behaviour and an
  operational liveness limitation. It is **not** an application defect.
- **No architectural improvement is recommended.** The options Stage37
  listed (a shorter user timeout; thin-stream linear timeouts, which do
  not apply at 5 segments in flight) trade correctness for speed, and
  need separate approval.
- **The one related improvement is 7.1,** which brings the relay's
  *other* socket under the same bound.

### 7.4 Transport abstraction (A3)

Session forwarding exists in three hand-written forms:

- **Encrypted TCP:** framed AEAD, `try_join!`, half-close by EOF
  ([`secure_copy_server_side`](../crates/werewolfd/src/transport/tcp_encrypted.rs)).
- **QUIC:** `tokio::io::copy` both ways plus `finish` or `reset`,
  written inline inside a `tokio::select!` in the accept loop
  ([`quic.rs:176–262`](../crates/werewolfd/src/transport/quic.rs)).
  rustfmt does not format inside the macro, so about 85 dense lines get
  no formatting.
- **Plain TCP:** [`tcp_plain.rs`](../crates/werewolfd/src/transport/tcp_plain.rs).

The target-side rules (reset-on-close arming, lease cancellation, shutdown
propagation) are duplicated and must be kept in step by hand. 7.1 shows
the cost: one socket rule was applied on one side only.

**Kind:** technical debt, MEDIUM. **Recommendation:** not a refactor
stage now. Extract the *target-connection setup* (connect, arm,
configure) into one function as part of Stage39, and leave the copy
loops alone.

## 8. Routing, discovery and privacy findings

These are research directions, not approved features. Each is evaluated
against benefit, attack surface and testability.

| Direction | Assessment | Verdict |
|---|---|---|
| Multi-relay circuits | Real benefit against a single malicious relay linking requester and destination. But it needs a circuit protocol, key layering, relay selection, and an adversary model (global observer? colluding relays?). It changes the threat model and the wire contract. | defer; it needs its own threat-model survey first |
| Better path selection, relay resilience | Today: one operator relay, no failover between relays. Failover would duplicate Stage27's provider logic at the route level. It is modest and testable, but it depends on 7.1 to detect a dead relay path at all. | defer until after Stage39 |
| Metadata privacy (padding, cover traffic) | Frame padding exists (`write_encrypted_frame`: random 768–2048-byte buckets); timing and volume are visible by nonclaim. Cover traffic costs bandwidth and needs an adversary model. | defer; nonclaims stay |
| Discovery resilience, gossip | Pull-only is documented, and eclipse by withholding is a stated limit. Gossip adds attack surface. | defer |
| Traffic observation, distributed defensive intelligence | Would require inspecting or sharing traffic metadata, against the confidentiality goals | not recommended |
| Defensive resource management | Stage16 ceilings exist. 7.1 (stranded sessions) and S2 (initiator bounds) are the concrete gaps found. | covered by priorities 1 and 2 |

## 9. Operational findings

### 9.1 Missing implementation or missing evidence?

| Area | Missing implementation | Missing evidence |
|---|---|---|
| Relay-to-destination path loss | **yes, likely** (7.1) | yes |
| IPv6 | **yes for QUIC** (9.2) | yes |
| ARM and other targets | unknown (no target-specific code found) | yes: not built or run |
| Suspend/resume, real clock changes | unknown | yes |
| Indeterminate commit on real storage | no (unit-tested) | yes |
| Governance sync failures | no (pull with retry) | partly (gates) |
| Terminal lifecycle under storage faults; destruction under a full disk | no | yes |
| Repeated disruption | no | sandbox only (Stage32), not WAN |

### 9.2 IPv6

The QUIC initiator endpoint binds `0.0.0.0:0`
([`quic_fang.rs:138`](../crates/werewolfd/src/quic_fang.rs)), an
IPv4-only socket, so a QUIC Fang to an IPv6 Pack address cannot send.
This is SUPPORTED by the code and socket semantics; not tested. The
encrypted-TCP path uses `TcpStream::connect`, which follows the
address's family.

**Kind:** an operational limitation in the implementation, recorded so
far only as a test gap ("IPv6: not run"). No support claim should be
made until it is fixed and tested.

## 10. Code-quality findings

| Finding | Evidence | Kind | Priority |
|---|---|---|---|
| Duplicated forwarding and target-setup rules (7.4) | CONFIRMED | technical debt | MEDIUM (part handled in Stage39) |
| QUIC accept loop: about 85 unformatted lines inside `select!` | CONFIRMED | technical debt | LOW–MEDIUM (readability of security-relevant code) |
| Mutex-poisoning unwraps | CONFIRMED | acceptable idiom | none |
| No `deny(unsafe_code)` | CONFIRMED | hardening | LOW |
| Large modules: `control.rs` 2,576 lines, `control/mutation.rs` 2,695 | CONFIRMED | maintainability | LOW (test-heavy, cohesive) |

## 11. Testing gaps

| Property | Unit | Integration (lab) | Sandbox (Stage32) | Real WAN | CI |
|---|---|---|---|---|---|
| Authentication and authorization matrices | yes | yes (Python matrices) | — | yes (Stage22R) | **Rust unit only** |
| Relay to destination: path loss | no | no | **no** | **no** | no |
| Target failure (RST, crash, half-close) | yes | yes | — | yes (D07; through a relay in Stage26) | partly |
| IPv6 | no | no | no | no | no |
| Advisory audit | — | — | — | — | **no** |
| Python unit tests (64), acceptance (12), ten lab gates | — | yes | — | — | **no** |

The most valuable gaps are relay-to-destination path loss (a plausible
defect, never exercised) and CI coverage of the Python and advisory
checks (their regressions are currently caught only at stage gates). Test
volume is not the issue.

## 12. Prioritised opportunity matrix

Each criterion is scored 1–5 and weighted: security 30%, reliability
25%, user/operator benefit 15%, architecture 15%, feasibility 10%,
testability 5%.

| # | Opportunity | Sec | Rel | User | Arch | Feas | Test | **Score** | Complexity | Risk |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | Relay and target-connection liveness | 3 | 5 | 4 | 3 | 5 | 5 | **3.95** | LOW | LOW |
| 2 | QUIC initiator resource-bound parity | 4 | 2 | 2 | 3 | 4 | 4 | **3.05** | LOW–MEDIUM | MEDIUM (throughput; must not mask S3-P2) |
| 3 | CI coverage: Python suites, selected gates, pinned advisory audit, `--locked` tests | 3 | 3 | 2 | 2 | 5 | 5 | **3.00** | LOW | LOW |
| 4 | Routed-QUIC regression root cause, plus an upstream report | 1 | 4 | 3 | 3 | 3 | 3 | **2.65** | MEDIUM | LOW (diagnosis only) |
| 5 | IPv6 transport support and validation | 1 | 3 | 3 | 3 | 3 | 3 | **2.40** | MEDIUM | MEDIUM |
| — | Forwarding-core refactor (7.4) | 2 | 2 | 1 | 4 | 2 | 3 | 2.20 | MEDIUM | MEDIUM |
| — | Relay failover | 1 | 3 | 3 | 3 | 2 | 3 | 2.30 | HIGH | MEDIUM |
| — | Multi-relay privacy circuits | 3 | 1 | 3 | 5 | 1 | 1 | 2.50 | VERY HIGH | HIGH |

**Why these scores:**
- **#1:**
  - **Reliability 5:** it closes the only documented never-exercised
    path with a code-level cause.
  - **Security 3:** availability only; quota-bounded; no bypass.
  - **Feasibility 5:** reuses existing constants and one function.
- **#2:** security 4, because it is a resource bound against
  authenticated peers, but it touches the same configuration surface as
  the S3-P2 regression, so its risk is MEDIUM.
- **Privacy circuits** (2.50, above IPv6 by arithmetic): they score on
  architecture but fail feasibility (1) and testability (1). They are
  VERY HIGH complexity, HIGH risk, and change the threat model, so the
  aggregate does not override those (handoff section 12).

## 13. Top five recommendations

1. **Relay and target-connection liveness.**
   - *Limitation:* target sockets lack the session liveness bounds (7.1).
   - *Evidence:* CONFIRMED in code; the consequence is a HYPOTHESIS.
   - *Modules:* `transport/tcp_encrypted.rs`, `transport/quic.rs`,
     `authority.rs` (`connect_poll` callers).
   - *Benefit:* relay and receiver sessions over a dead destination path
     end within the existing bounds, and quotas recover.
   - *Security:* neutral; no new timeouts; reset semantics kept.
   - *Complexity:* LOW.
   - *Tests:* sandbox relay-to-destination blackhole cells, plus
     regression of the affected Stage32 surfaces and the D07 equivalents.
   - *Dependencies:* none.
   - *Recommendation:* **Stage39.**
2. **QUIC initiator resource-bound parity.**
   - *Limitation:* the initiator runs quinn defaults (S2).
   - *Modules:* `quic_fang.rs`.
   - *Benefit:* symmetric, documented bounds.
   - *Security:* positive (memory exposure to authenticated peers).
   - *Complexity:* LOW–MEDIUM.
   - *Tests:* resource tests; the QUIC Stage32 subset; S2-P2 and S3
     cells; and an explicit check that S3-P2 is not used as acceptance
     (it stays EXCLUDED).
   - *Dependencies:* an owner decision on values (window sizes are
     resource bounds, Stage34).
   - *Recommendation:* a separate stage after Stage39.
3. **CI coverage.**
   - *Limitation:* only Rust unit tests in CI.
   - *Modules:* `.github/workflows/rust.yml`.
   - *Benefit:* authorization-matrix and transport-policy regressions
     caught per PR.
   - *Security:* positive.
   - *Complexity:* LOW.
   - *Tests:* the CI itself.
   - *Dependencies:* the gates that need network namespaces cannot run
     on hosted runners, so only the unit and acceptance suites can.
   - *Recommendation:* a small standalone PR, at any time, with owner
     approval.
4. **Routed-QUIC regression root cause.**
   - *Limitation:* the Stage34 cause is unproven.
   - *Modules:* none in production (a disposable diagnostic build).
   - *Benefit:* a remediation path, or a precise upstream report.
   - *Complexity:* MEDIUM.
   - *Tests:* Stage34's cell method.
   - *Dependencies:* none; it is independent of Stage39.
   - *Recommendation:* a diagnostic stage after Stage39, or the upstream
     report alone.
5. **IPv6.**
   - *Limitation:* the QUIC initiator is IPv4-only (9.2), and nothing is
     tested.
   - *Modules:* `quic_fang.rs`, address parsing, Stage32 sandbox.
   - *Benefit:* real deployments on IPv6.
   - *Complexity:* MEDIUM.
   - *Tests:* new IPv6 sandbox cells for both transports.
   - *Recommendation:* its own stage; no support claim before it.

## 14. Preferred next stage

**Stage39: relay and target-connection liveness.** Why it is more
valuable now than the alternatives:

- **It is a problem, not a feature.** It has a code-level cause, sits on
  a documented never-exercised path, and affects a security-relevant
  resource (relay quotas).
- **It is bounded and verifiable.** It needs one setup function, the
  existing constants, and new sandbox cells. Its rollback is one revert.
- **It does not touch the S3-P2 or QUIC-regression surface.** It uses no
  new windows or timeouts, so it cannot blur Stage34 or Stage37.
- **It unblocks later work.** Relay failover or path selection cannot be
  built on a relay that cannot detect a dead destination path.

## 15. Proposed Stage39 contract (for owner review)

- **Problem:** the receiver's target connection, which on a relay is the
  WAN hop to the destination, has no keep-alive or user timeout, unlike
  every session socket (7.1).
- **Objective:** give every target connection the same kernel liveness
  bounds as session sockets, and show, in the sandbox, that a dead
  relay-to-destination path ends routed sessions within those bounds,
  with resets and recovered quotas.
- **Non-goals:**
  - no new or changed timeouts, windows or limits;
  - no application idle timeout;
  - no change to QUIC configuration, S3-P2, Stage37 conclusions or any
    historical verdict;
  - no relay failover, multi-relay or routing feature;
  - no forwarding-loop refactor;
  - no tokio change.
- **Components:** `transport/tcp_encrypted.rs` (`server_handshake`
  target connect), `transport/quic.rs` (target connect),
  `transport/mod.rs` (one shared target-setup function). Possibly
  `tcp_plain.rs`, by owner decision S39-D3.
- **Constraints and invariants:**
  - authorization order unchanged;
  - reset-on-close arming kept;
  - the lease gate unchanged;
  - the target connect still goes through `lease.connect_poll` over the
    already-authorized numeric addresses;
  - no change to the wire or CLI contract.
- **Milestones:**
  - **M1:** a failing sandbox reproduction of 7.1 (test first) on
    current `main`, to establish whether the hypothesis holds. If it
    does not reproduce, Stage39 stops and records that.
  - **M2:** the fix.
  - **M3:** regression: the affected Stage32 cells (S1, S2 and S3 at P0,
    P2 TCP and P5) and the D07 target-failure equivalents in the sandbox.
  - **M4:** a docs-only record and the `KNOWN_LIMITATIONS` update, only
    if supported.
- **Required tests:**
  - new sandbox cells: relay-to-destination blackhole and hard path loss
    during (i) an idle session, (ii) an upload and (iii) a download, for
    QUIC and TCP hops;
  - checks in each cell:
    - the relay releases its lease and sockets within the keep-alive or
      user-timeout bound plus `CLEANUP_WINDOW`;
    - the requester's application sees a reset, never a clean EOF;
    - no fd, thread or RSS leak (Stage32 item 5);
    - per-peer quota restored;
  - unit tests that the target socket carries the options.
- **Acceptance criteria:**
  - every new cell PASSES on the fix;
  - the M1 reproduction fails on `main` (or Stage39 stops at M1);
  - every existing gate and suite passes;
  - S3-P2 stays EXCLUDED and is not used as evidence.
- **Risks:**
  - a legitimate target unreachable for more than 60 s now loses its
    session, as every Werewolf hop already does;
  - keep-alive probes to target hosts become observable traffic, so the
    nonclaims gain one line.
- **Rollback:** revert the single fix commit; the constants are
  unchanged.
- **Harness note:** adding the new cells changes the chaos harness's
  scenario list, never an existing rule or verdict. It needs the owner's
  approval as part of this stage (S39-D4).

## 16. Alternatives rejected or deferred

- **Rejected:** the forwarding-core refactor as a stage, because its
  value is mostly realised by Stage39's single setup function; traffic
  inspection or shared defensive intelligence, which conflicts with
  confidentiality.
- **Deferred:**
  - QUIC initiator bounds (priority 2) and IPv6 (priority 5): separate
    architectural decisions;
  - QUIC regression diagnosis (priority 4): independent and
    diagnostic;
  - CI coverage (priority 3): a small PR, not a stage;
  - multi-relay circuits, relay failover, metadata privacy and gossip:
    they need threat-model surveys first;
  - ARM and other platform validation: evidence-only, no code finding;
  - suspend/resume and real-storage validation: evidence-only.

## 17. Risks

- **The hypothesis may not reproduce:** M1 guards this, and Stage39
  stops if it does not.
- **Interaction with application targets:** applying the bounds to
  every target, not only relay targets, changes behaviour for
  application targets (S39-D1).
- **Sandbox only:** the sandbox cells are not a real WAN; a WAN check is
  optional, by owner decision.

## 18. Owner decisions

| # | Decision | Options | Recommended |
|---|---|---|---|
| S39-D0 | Approve Stage39 as scoped | approve, amend, reject | approve |
| S39-D1 | Which target connections get the bounds | all targets; non-loopback targets only; relay targets only (needs a signal the receiver does not have today) | all targets, the simplest and most uniform |
| S39-D2 | Values | the existing session constants (keep-alive 30 + 10 × 3, user timeout 60 s); anything else | existing constants, no new timeouts |
| S39-D3 | Plain TCP (`tcp_plain.rs`) included | yes; no | no (explicit opt-in, out of the secure path) |
| S39-D4 | New sandbox cells in the chaos harness (scenario list only; rules unchanged) | approve; diagnostic-only scripts outside the harness | approve |
| S39-D5 | A WAN check of the relay-to-destination path | yes; no | optional, after the sandbox PASS |
| — | Later stages: QUIC initiator bounds, CI coverage PR, QUIC regression diagnosis or upstream report, IPv6 | order | after Stage39: CI PR at any time; then QUIC initiator bounds; IPv6 |

**Not authorised by this survey:** implementing Stage39, any production,
dependency or harness change, tokio promotion, and release certification.
