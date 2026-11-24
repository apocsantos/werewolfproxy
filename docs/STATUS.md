# WerewolfProxy Status

## Release

**1.0.0-rc.2**: release candidate for Linux x86_64
(`x86_64-unknown-linux-gnu`). The tag `v1.0.0-rc.2` and its archive
(`62d35c13…89a3`) are frozen by the
[Stage33 candidate audit](STAGE33_CANDIDATE_AUDIT.md), with the #92 and
#93 dependency remediations (rustls 0.23.46, rustls-webpki 0.103.16,
quinn-proto 0.11.18).

**1.0.0-rc.1**, the previous candidate: the tag `v1.0.0-rc.1` and its
artifacts stay frozen by the [Stage21 audit](STAGE21_RC1_AUDIT_FREEZE.md).

## Current development security status

Stage50 dependency remediation is on `main` through owner-merged
[PR #159](https://github.com/apocsantos/werewolfproxy/pull/159) (`c1e7845`,
2026-11-16): quinn-proto 0.11.19 and quinn-udp 0.5.16. Workspace/proof
audits, 460 Rust tests and 13/13 local gates passed. This resolves the
Stage49 audit blocker. Hosted Rust CI and CodeQL could not start because
GitHub reported an account billing lock at that time. Hosted Rust CI was
available again for PR #185; its run passed the Rust suite, disposable
acceptance suite, formatting, Clippy and release build. The
[Stage50 closure](STAGE50_CLOSURE.md) records scope and remaining limits;
the frozen releases above are unchanged.

## Current development hostname behaviour

Stage52 bounded QUIC hostname refresh is on `main` through owner-merged
[PR #163](https://github.com/apocsantos/werewolfproxy/pull/163) (`c9e90c8`),
with the [M3 regression](STAGE52_M3_REGRESSION.md) merged in
[PR #164](https://github.com/apocsantos/werewolfproxy/pull/164) (`3aeb832`,
2026-11-17). New hostname connections through direct, routed and failover
QUIC Fangs resolve again without reopening. Open-time validation, key
pinning, provider order and the verified-ACK commitment boundary remain.
Capacity wait, lookup and address racing share the existing 5 s dial
deadline; new refresh work has a 16-job daemon-wide cap.

The merged runtime passes 474 Rust tests, 21/21 migration runs, 39/39
resolver scenarios, the address/refusal/resource/P0 controls and 13/13
local gates. Workspace/proof audits pass against the recorded RustSec
snapshot `233155b4`. All 11 hosted jobs on the M3 head were prevented
from starting by the account billing lock; no hosted pass is claimed.
The [Stage52 closure](STAGE52_CLOSURE.md) records delivery and the
[remaining limitations](../KNOWN_LIMITATIONS.md): system resolver caches,
blocked-call lifetime, shared-capacity saturation and local-only DNS
fixtures. Frozen releases, S3-P2/S2-P4 exclusions, Stage38 closure and
Tokio 1.52.3 are unchanged.

## Current development peer-session admission

Stage53 reserved peer-session shares are on `main` through owner-merged
[PR #167](https://github.com/apocsantos/werewolfproxy/pull/167) (`b1fc389`),
with [M3 regression](STAGE53_M3_REGRESSION.md) merged in
[PR #168](https://github.com/apocsantos/werewolfproxy/pull/168) (`d3758e9`,
2026-11-18). One daemon coordinates authenticated outgoing Fangs by the
full receiver key across QUIC/TCP: two get 32 reservations each, three
get 22/21/21 from a local 64-session budget. Routed attempts reserve both
relay and destination. Local denial may advance failover before verified
ACK; established sessions retain their provider and are never replayed.

M3 passes 485 Rust tests, 39/39 membership cases, 12/12 two-Fang cases,
15/15 provider/activation controls, the resolver/address/resource/P0
regressions and 13/13 local CI gates. All 26 campaign workers recorded
successful actual exits. All 11 hosted jobs on the M3 head were prevented
from starting by the account billing lock; no hosted pass is claimed.

The Stage53 M1–M4 work is owner-merged; the [closure](STAGE53_CLOSURE.md)
records unused reserved capacity,
the 64-member activation bound and no bounded admission after late
activation. Its smaller-share idle boundary is addressed by Stage54 below.
This remains fairness between configured Fangs in one daemon, not a per-user
or saturation-service guarantee. Receiver authority and frozen releases
remain unchanged.

## Current development idle reclamation

Stage54 bounded idle reclamation is on `main` through owner-merged
[PR #171](https://github.com/apocsantos/werewolfproxy/pull/171) (`a8bcc12`),
with [M3 regression](STAGE54_M3_REGRESSION.md) merged in
[PR #172](https://github.com/apocsantos/werewolfproxy/pull/172) (`4ff2fee`,
2026-11-21). On local peer-capacity denial, one useful committed session
in the same Fang, idle for at least 60 s, may be cancelled. Actual held
receiver keys and current share/total checks determine usefulness; real
cleanup releases quota. One nonblocking operation per Fang waits at most
1 s within existing deadlines. The one-victim budget includes earlier
local/source eviction and subsequent provider attempts.

Eligible idle primary capacity is reclaimed before advancing to fallback;
active saturation retains fallback. Verified destination ACK commits the
provider, with no replay after failure. Sibling/recent/setup work is
protected, and there is no forced multi-victim rebalance or idle expiry.

M3 passes 497 Rust tests, all 31 campaign workers, 34 Stage54 native labs,
the resolver/address/resource/fairness controls, twelve S2/S3/S4/S7 P0
trials and 13/13 local CI gates. All **12 hosted checks** on `0a61c8d`
pass, including Rust CI and CodeQL with no new alerts. Earlier stages'
billing-blocked hosted records describe their original publication dates.

The [closure handoff](STAGE54_CLOSURE.md) and
[current limitations](../KNOWN_LIMITATIONS.md) record the compatibility
cost: a legitimate idle session can reset below 64 local slots. Admission
can still lose a cleanup/capacity race after one victim; active saturation,
unused reserved shares and late over-share membership remain limits.
Validation is local, with exact membership/slow-cleanup wait cases covered
by deterministic unit tests. Frozen releases, S3-P2/S2-P4 exclusions,
Stage38 closure, Tokio 1.52.3 and advisory policy remain unchanged.

## Current development global idle reclamation

Stage55 is closed; M1–M4 are owner-merged through [M4 PR #177](https://github.com/apocsantos/werewolfproxy/pull/177)
(`406f68a`). Owner-merged [M1](STAGE55_GLOBAL_IDLE_ADMISSION_SURVEY.md),
[M2](STAGE55_M2_GLOBAL_RECLAMATION.md) and [M3](STAGE55_M3_REGRESSION.md)
records document the policy and validation. M2 PR #175 (`16d67a5`) added bounded same-Fang
reclamation when a Fang with local room is denied by the daemon-wide 1024
session cap. One useful established session idle for at least 60 s may be
cancelled; actual cleanup releases capacity, and a single bounded retry stays
within the original deadline and shared victim budget. Empty Fangs remain
denied when siblings fill the cap; no cross-Fang eviction or active-session
preemption is added.

M3 PR #176 (`80c86a5`) passes nine direct, two routed, two provider-fallback,
one source-bound and three concurrent full-cap cases, all 13 local-CI gates,
and the retained regression campaign (513 Rust tests). All hosted checks on
the merged head pass, including Rust CI, CodeQL, formatting, Clippy and release
build. Full-cap source validation is one TCP loopback-namespace case; active
primary/provider commitment paths remain covered below the cap by Stage54.
Frozen releases and the documented fairness, idle-reset, race and WAN limits
remain unchanged.

## Local Fang client fairness

The active-loopback flood remains an explicit TCP limitation. Stage56 is closed:
M1–M4 are owner-merged through [M4 PR #182](https://github.com/apocsantos/werewolfproxy/pull/182)
(`ec30713`). The optional Linux Unix endpoint uses `SO_PEERCRED` UID identity
in the shared 16-session source share; `service connect --local-endpoint`
provides the public client path while legacy TCP behavior remains unchanged.
M4's five-path exact-byte gate and UID 0/UID 1 namespace test passed, and all
hosted Rust CI jobs passed. See the [M1 survey](STAGE56_LOCAL_FANG_IDENTITY_SURVEY.md),
[M2 design](STAGE56_M2_LOCAL_UNIX_DESIGN.md), [M3 implementation](STAGE56_M3_UNIX_ENDPOINT_IMPLEMENTATION.md),
[M4 regression](STAGE56_M4_UNIX_TRANSPORT_REGRESSION.md) and [closure](STAGE56_CLOSURE.md).

## Hosted CI coverage

PR #185 adds the Python integration unit tests and disposable checkout
acceptance runner to the hosted Rust CI test job. The hosted run passed both
additions alongside the existing Rust tests, formatting, Clippy and release
build. PR #186 closed the Stage57 coverage work. PR #187's hosted checks also
passed. Namespace/netem, multi-UID and WAN campaigns remain outside hosted
CI. See the [coverage record](CI_COVERAGE_SPEC.md).

## Current routed QUIC loss limitation

Stage58's survey records upstream Quinn PR #2976 as a plausible lead for the
Stage34 S3-P2 failure under packet loss. The upstream PR remains open and the
latest published `quinn-proto` remains 0.11.19 as checked on 2026-11-24.
S3-P2 stays **EXCLUDED**; there has been no dependency or production
transport change and no retest. A future candidate must be released,
security-reviewed against Stage50, and pass Stage34's exact-sequence
regression check before the separately gated certification campaign.
See the [Stage58 survey](STAGE58_QUIC_GAP_BUFFER_SURVEY.md).

## Certified

| Area | Records |
|---|---|
| Target authorization | Stage9 |
| Replay hardening | Stage10 |
| Transport, local and session security | Stage11A–C |
| Receiver authentication and wire opacity | Stage12 |
| Traffic morphology (documented, not hidden) | Stage13 |
| Cryptographic hygiene | Stage14 |
| Persistent-state anti-rollback and manifest | Stage15, Stage15B |
| Pre-auth resource bounds | Stage16 |
| Linux service lifecycle, CLI, packaging | Stage17–19 |
| Adversarial soak | Stage20 |
| RUSTSEC-2026-0285 remediation | Stage20S |
| RC1 audit and freeze | Stage21 |
| rc.2 candidate audit and freeze | Stage33 |

Supported transports are QUIC and encrypted TCP; plain TCP is an explicit
opt-in only. See [transport architecture](../TRANSPORT_ARCHITECTURE.md).

## Post-RC1 validation (Stage22 / Stage22R)

Stage22 ran on one host against the exact RC1 archive. Stage22R ran over the
real WAN between an initiator and a remote Debian VPS receiver:

| Record | Scope | Result |
|---|---|---|
| [Stage22](STAGE22_PRIVATE_OPERATIONAL_VALIDATION.md) | Private operational validation of the exact RC1 archive | Certified; host reboot not tested |
| LC-01 | Fail closed on Pack endpoint change; await Fang quiescence | Fixed on `main` |
| [HC02](STAGE22R_HC02_LOCAL_CANDIDATE.md) | Encrypted TCP full-duplex cancellation safety | Fixed, WAN certified |
| [QUIC WAN](STAGE22R_QUIC_WAN_CERTIFICATION.md) | Sustained full-duplex QUIC over WAN | Certified |
| [D01](STAGE22R_D01_RECEIVER_RESTART_CERTIFICATION.md) | Receiver restart | Certified |
| [D02](STAGE22R_D02_INITIATOR_RESTART_CERTIFICATION.md) | Initiator restart | Certified |
| [D03](STAGE22R_D03_HARD_CRASH_CERTIFICATION.md) | Initiator hard crash (SIGKILL) | Certified |
| [D04](STAGE22R_D04_RECEIVER_HARD_CRASH_CERTIFICATION.md) | Receiver hard crash (SIGKILL) | Certified |
| [D05A](STAGE22R_D05A_HARD_PATH_LOSS_CERTIFICATION.md) | Hard WAN path loss | Certified |
| [D05B](STAGE22R_D05B_TEMPORARY_BLACKHOLE_CERTIFICATION.md) | Temporary WAN blackhole, same-session survival | Certified |
| [D06](STAGE22R_D06_REPEATED_LOSS_RECOVERY_CERTIFICATION.md) | Repeated WAN blackholes and hard path loss without restart | Certified on `b911e41` |
| [D07](STAGE22R_D07_TARGET_FAILURE_CERTIFICATION.md) | Target failure mid-session (RST, crash, half-close, absent) | Certified on `b911e41` |
| [D08](STAGE22R_D08_RESOURCE_PRESSURE_CERTIFICATION.md) | Resource pressure at production ceilings (sessions, stalled TCP/QUIC, RPC rate, slow targets) | Certified on `bcf7bbd` |
| [D09](STAGE22R_D09_STORAGE_FAULT_CERTIFICATION.md) | Storage faults fail closed (full, unwritable, read-only, torn state) | Certified on `b911e41`; indeterminate commit unit-tested only |

The HC02 and LC-01 fixes are on `main` after the RC1 tag, so they are not in
the 1.0.0-rc.1 archive.

## Development on `main` after RC1 (Stage23–26)

None of this is in the 1.0.0-rc.1 archive.

| Stage | Scope | State |
|---|---|---|
| Stage23 | Named services: `service add/list/show/remove/connect`, `services.json` | Implemented; lab gate `service_gate.py` |
| [Stage24](STAGE24_FINDNODE_DESIGN.md) | Pack-only peer discovery: `ping` and `find_node` RPCs, signed contact records, multi-hop lookup | Implemented; lab gate `discovery_gate.py`; WAN checked |
| [Stage25](STAGE25_SERVICE_DISCOVERY_DESIGN.md) | Service discovery: `service publish`, `discover services`, `service add --from`; visibility follows Target Policy | Implemented; lab gate `service_discovery_gate.py`; WAN checked |
| [Stage26](STAGE26_MULTIHOP_ROUTING_DESIGN.md) | Multi-hop routing through one relay, end-to-end identity and authorization; `route set/list/remove` | Implemented; lab gate `routed_fang_gate.py`; WAN checked (three defects found and fixed) |
| [Stage27](STAGE27_PROVIDER_SELECTION_DESIGN.md) | Provider selection: ordered providers per service, per-connection failover before establishment only | Implemented; lab gate `provider_gate.py`; WAN checked |
| [Stage28](STAGE28_GOVERNANCE_SURVEY.md) | Network governance: 7-of-10 signed statements (genesis, key rotation, network revocation), offline signing tool, chain head in the protected manifest (schema 2), fork fencing | Implemented; lab gate `governance_gate.py`; no network component to WAN-check (statements are operator-delivered) |
| [Stage29](STAGE29_ACTIVATION_DESIGN.md) | Admission and activation: network roster (at most 1024), one-way activation as a chain fact, locally derived admission with a fence for unadmitted nodes, `pack add` bounded by the roster, permanent revocation; manual `governance sync` from Pack peers with conflict fencing | Implemented; lab gates `activation_gate.py`, `sync_gate.py`; WAN checked (sync through a relay, disposable daemons only) |
| [Stage30](STAGE30_LIFECYCLE_SURVEY.md) | Network lifecycle: chain, node and capability states; DEGRADED vs UNRECOVERABLE criteria; 7-of-10 death statement and local retirement; destruction, crash semantics and the Tombstone | Decisions accepted 2026-10-26; [spec](STAGE30_LIFECYCLE_SPEC.md) approved 2026-10-26; steps 1–2 implemented (death statement, key check, Tombstone; destruction, `den retire`, exit 67); lab gate `death_gate.py`; real-host check passed 2026-10-26 (step 3) |
| [Stage31](STAGE31_AUTO_SYNC_SURVEY.md) | Automatic governance sync (Stage29 G4 follow-up): when it runs, local off switch, schedule and jitter, peers, which statements apply unattended, operator precedence, death boundary, status | Decisions accepted 2026-10-26 (1a, 3a, 6a, 7a with owner corrections; 2a, 4a, 5a, 8a, 9); [spec](STAGE31_AUTO_SYNC_SPEC.md) approved 2026-10-27 (S1–S5); step 1 implemented (switch, scheduler, operator precedence, status); lab gate `auto_sync_gate.py`; real-host check passed 2026-10-27 (step 2) |
| [Stage32](STAGE32_ROBUSTNESS_SURVEY.md) | Robustness and chaos certification: netem loss, jitter, reordering, duplication and corruption in unprivileged network namespaces; wall-clock jumps and process stalls; PASS criteria | Decisions accepted 2026-10-27 (1a, 2a, 3a, 5a, 6a, 7a); faketime acceptance check passed; [spec](STAGE32_ROBUSTNESS_SPEC.md) approved 2026-10-27 (X1–X5); harness `chaos.py` and dry run done (step 2); network campaign paused at S1-P5 (owner: EXCLUDED, invalid bound; P5 payloads now calibrated against raw TCP); F1 (encrypted-TCP setup under loss): a defect, fixed by one initiator setup retry before the challenge; F2 (sessions under burst loss): a documented limit, `TCP_USER_TIMEOUT` stays 60 s; harness echo idle timeout fixed, S1-P3 runs EXCLUDED; campaign resumed 2026-10-27 with the F1 fix; S2-P4 (F3, QUIC throughput under reordering) EXCLUDED as a documented limit; S3-P2 EXCLUDED (invalid workload), S3 payloads calibrated on the relayed path (KiB allowed), rerun PASS; P3 transfer availability a documented limit (S1/S2/S3-P3 EXCLUDED); S3-P4 EXCLUDED under F3; owner rule: a routed QUIC hop under jitter/reordering that reaches the bound while demonstrably progressing, with no corruption, crash or security failure, is EXCLUDED under F3 (harness-enforced); S3-P5 EXCLUDED under F3; F4 (`TCP_NODELAY` on encrypted-TCP sockets) merged; F5 (QUIC OPEN lost when reordered ahead of the initiator's Finished): a defect, fixed (the initiator waits for handshake confirmation); campaign completed 2026-10-29 from S3-P6 on main with F4 and F5: all PASS except S6-P2 run 3 (EXCLUDED, harness fenced before the session reached B); S6 harness fixed and S6 rerun 9 of 9 PASS, S6 certified (owner decision); every network cell has a verdict; [time campaign](STAGE32_TIME_CERTIFICATION.md): T1 (wall-clock jumps under libfaketime) certified 9 of 9, contact records refused beyond ±7 days as the documented limit (owner decision on -8d); T2 (SIGSTOP/SIGCONT stalls) certified 9 of 9, overdue automatic sync runs once on resume, never a burst; Stage32 complete 2026-10-29, KNOWN_LIMITATIONS updated |
| [Stage33](STAGE33_CLI_CANDIDATE_SURVEY.md) | CLI architecture-complete candidate: what "complete" means, candidate form and name, consolidated CLI and wire contracts, hidden commands, documents, gates, upgrade from rc.1, scope | Decisions accepted 2026-10-29, all as recommended (1a, 2a `1.0.0-rc.2` with 2-i, 3a, 4a, 5a, 6a, 7a); step 2 done (Cargo package versions `1.0.0-rc.2`); step 3 done (documents: README, QUICKSTART, INSTALL, SECURITY, OPERATIONS, the consolidated CLI contract and every v1 contract, ARCHITECTURE, TRANSPORT_ARCHITECTURE, KNOWN_LIMITATIONS); gates changed to 6b (no WAN smoke); dependency remediations #92 and #93; step 4 done, [candidate audit](STAGE33_CANDIDATE_AUDIT.md) PASS (archive `62d35c13…89a3`, reproducible; upgrade from rc.1 checked); step 5 done: tag `v1.0.0-rc.2` on `b192158` (2026-11-01), frozen |
| [Stage34](STAGE34_S3_P2_RECALIBRATION_SURVEY.md) | S3-P2 recalibration: the routed `loss 10%` workload marginal on every build (Stage33); stricter calibration, run count, scope, build, failure handling | Decisions accepted 2026-11-01 (1a–5a); payload recalibrated to 512 KiB (#97); owner kernel-tail rule for the routed TCP hop (#98); quinn-proto 0.11.18 regression of the routed QUIC hop under loss found and bisected; **S3-P2 EXCLUDED**, quinn-proto and the send window unchanged (owner); closed 2026-11-01, [record](STAGE34_S3_P2_RECORD.md) with retest requirements |
| [Stage35](STAGE35_TOKIO_1_54_1_TRIAL.md) | tokio 1.54.1 remediation trial (RUSTSEC-2026-0386) | Trial 2026-11-01: advisories, fmt, clippy, 412 tests, Python, acceptance, ten lab gates and Stage12 proofs PASS; S3-P2 regression gate STOP (one TCP-hop failure outside the kernel-tail rule, two kernel tails); **rejected**, tokio stays 1.52.3 with the advisory accepted; owner review pending |
| [Stage36](STAGE36_TOKIO_AB_DIAGNOSIS.md) | tokio 1.52.3 vs 1.54.1 controlled A/B diagnosis of S3-P2 (after the Stage35 trial, PR #100) | Diagnosis 2026-11-02, 120 runs (instrumented ABBA, uninstrumented BAAB): TCP no material regression detected, the Stage35 `CLOSE-WAIT`-at-bound FAIL reproduced on tokio 1.52.3; QUIC no material regression detected; tokio 1.54.1 accepted for further certification, subject to a Stage37 owner decision; tokio stays 1.52.3 on `main` |
| [Stage37](STAGE37_RELAY_BACKPRESSURE_DIAGNOSIS.md) | Encrypted-TCP relay backpressure and shutdown diagnosis: why Y stays in `CLOSE-WAIT` until the S3-P2 bound | Diagnosis 2026-11-02 on `main` `4bcaaee` (tokio 1.52.3) with lifecycle tracing in a disposable build, 30 runs: the `CLOSE-WAIT`-at-bound FAIL reproduced and traced (run 17); **Outcome A, correct behaviour under kernel backpressure**: the relay waits in one frame write on a never-writable socket while retransmissions under exponential backoff are lost, ended by `TCP_USER_TIMEOUT`; no application defect, no missed wakeup; no rule, code or dependency change; Stage38 comparative criteria proposed for owner decision |
| [Stage38](STAGE38_COMPARATIVE_CERTIFICATION_SURVEY.md) | Comparative transport certification architecture: absolute Layer A (security, correctness, resources) and comparative Layer B (documented liveness limitations, non-inferiority against a contemporaneous control); decisions D1–D9 | Survey 2026-11-02 on `main` `c5400ae` (#103); owner decisions 2026-11-02: D1 final (raw outcomes authoritative; Layer A adds `HELD`; separate `LIVENESS_*` and `WORSENING_DETECTED` findings); D2, D7, D8 approved; D3 conditional on pilot evidence; D4 provisional (E1 7.5 pp, E2 10 pp); D5 not yet approved as a certification method; D6 conditional on simulation; D9 approved with the controlled 9b emergency exception; scope the S3-P2 encrypted-TCP hop only, kernel tails counted in E1; historical probability explanation corrected; [statistical validation spec](STAGE38_STATISTICAL_VALIDATION_SPEC.md) (#105); M3 offline simulation 2026-11-02 ([report](STAGE38_M3_SIMULATION_REPORT.md), `tools/stage38_statistics/`): `NO_VALIDATED_PRIMARY_METHOD` that is certification-ready; C0–C5 and C7 rejected (false acceptance under block dependence); C6 controls error but needs about 48–64 quartets (61–90 h) at δ 7.5 pp; D4 and D5 still not approved; A/A pilot not authorised; M3b rare-event research 2026-11-03 ([report](STAGE38_M3B_RARE_EVENT_RESEARCH.md)): **M3B-B**, `NO_PRACTICAL_VALIDATED_METHOD`; no new method (betting, empirical Bernstein, bootstrap-t, studentised randomisation) improves on C6; a proven floor of 39 candidate blocks at δ 7.5 pp for any method without a within-block dependence assumption; C6 confirmed across G1–G3 but still about 48 quartets; comparative certification stays suspended; **Stage38 M3b CLOSED** 2026-11-03 ([closure](STAGE38_M3B_CLOSURE.md)): comparative certification suspended, no practical validated primary method, no M4 authorisation; D4 provisional, D5 not approved, V5 cap stays 2; no implementation, rule, dependency or verdict change; tokio stays 1.52.3, S3-P2 stays EXCLUDED |
| [Stage39](STAGE39_ENGINEERING_PRIORITY_SURVEY.md) | Post-Stage38 engineering priority survey; proposed Stage39: relay and target-connection liveness (the receiver's target socket, the relay's WAN hop to the destination, lacks the session sockets' keep-alive and user timeout) | Survey 2026-11-03 on `main` `e18fec9` (#109); decisions S39-D0–D5 (M1 only); **M1 reproduction 2026-11-03: CONFIRMED** ([record](STAGE39_M1_LIVENESS_REPRODUCTION.md)): on unchanged `main` `7b9f2c1`, in 14 of 14 valid sandbox runs (both hops, blackhole and link down, idle, upload, download), Y kept the session lease and target socket beyond the 150 s bound and 90 s after the client closed, while D's configured socket closed in 56–80 s; **M2 fix 2026-11-04: M2-PASS** ([record](STAGE39_M2_TARGET_LIVENESS_FIX.md)): both secure receivers apply the existing `configure_liveness` to the target connection before publication and the ACK; in the M1 sandbox matrix (22 runs) Y now releases the lease and target socket 55.9–80.9 s after the Y ↔ D failure (M1: held beyond 390 s), the requester sees a reset, healthy sessions unaffected; **M3 regression 2026-11-04: PASS** ([record](STAGE39_M3_REGRESSION.md)): ten lab gates and Stage12 proofs PASS; D07 target-failure equivalents in the sandbox 84 of 84 PASS (direct and routed, both transports); Stage32 target-connection cells PASS or their historical F3 EXCLUDED status (S2-P4 behaviour as recorded); `KNOWN_LIMITATIONS` records the rc.2 defect and the sandbox-only fix on `main`; WAN check not authorised; Stage38 stays CLOSED |

## Known gaps

See [known limitations](../KNOWN_LIMITATIONS.md). Main items:

- Only Linux x86_64 is certified.
- No anonymity, cover traffic or timing obfuscation; a Stage26 relay sees
  who talks to which listener, when and how much.
- No automatic peer pairing; public keys are exchanged out of band.
- Not run over a real WAN: netem-style random loss, jitter and reordering;
  host clock changes on a real machine. Stage32 exercised network impairments
  in local namespaces and clock jumps with `libfaketime`; Stage22R exercised
  restart, crash, path-loss, resource-pressure and storage-fault behaviour
  against later builds, not the frozen rc.1 archive.
- Routing has one operator-set relay per destination, with no route
  discovery or automatic selection. Path loss on both routed hops was
  exercised in sandbox namespaces. The relay-to-destination path has not
  been checked over a real WAN: Stage39-D5 was authorized but not run because
  the previously used VPS was unreachable.
- The shared transport trait, health scoring and automatic return to the
  preferred transport are not implemented.

## History

The pre-RC "operational beta" status page, including the script-era
transport policies and roadmap, is kept at
[history/STATUS_PRE_RC.md](history/STATUS_PRE_RC.md).
