# Stage53 — reserved peer-session share closure

**Status: M1–M3 merged; M4 closure for review, 2026-11-18.** The owner
merged the design, runtime and passing regression, then instructed
continuation. This record completes the approved documentation step on
baseline `main` `d3758e9ebad2c5b750ac6e23f6d52a591580481b`.

## Delivery and resulting behaviour

| Milestone | Owner-merged delivery | Record |
|---|---|---|
| M1 reproduction and Q2 contract | [PR #166](https://github.com/apocsantos/werewolfproxy/pull/166), `4fed878`, 2026-11-17 | [Design](STAGE53_PEER_QUOTA_FAIRNESS_SURVEY.md) |
| M2 reserved shares | [PR #167](https://github.com/apocsantos/werewolfproxy/pull/167), `b1fc389`, 2026-11-17 | [Implementation](STAGE53_M2_PEER_QUOTA_ADMISSION.md) |
| M3 membership and transport regression | [PR #168](https://github.com/apocsantos/werewolfproxy/pull/168), `d3758e9`, 2026-11-18 | [Results](STAGE53_M3_REGRESSION.md) |

Before Stage53, one Fang could fill the receiver's 64-session quota and
deny its already-open siblings, including across QUIC and encrypted TCP.
The accepted S53-D0–D6 contract implements Q2: one coordinator per daemon
groups authenticated outgoing Fangs by the full captured receiver key.
It partitions a local 64-reservation budget among registered members:
one gets 64, two get 32 each, three get 22/21/21. Remainders follow stable
registration order; unused shares are reserved and never borrowed.

Membership registers atomically before listener readiness, with at most
64 Fangs per receiver. A 65th registration prevents activation with
`FANG_PEER_CAPACITY`; failed activation rolls back registration. Prepared
and retiring members remain counted while their owned work or reservations
survive. Establishing and committed sessions retain reservations through
cleanup; empty coordinator entries are removed.

Direct attempts reserve the captured peer; routed attempts reserve relay
and destination together. Failover registers the union of potential
provider/route identities but reserves live capacity only for its current
attempt. A local denial resets a direct/routed client as `dial`; failover
may advance in existing provider order before verified ACK, releasing the
failed attempt first. After ACK, the selected demand set stays owned and
failure resets the client without replay to a fallback.

Adding a Fang recalculates shares without preempting established sessions.
An older Fang above its new share cannot admit more; a new Fang may have
to wait for old sessions to release capacity. Closing members increases
remaining shares only after their work ends. Receiver quotas and
authorization remain authoritative. Plain TCP, local/source/global
ceilings, idle policy, resolver coordination, deadlines, transport windows,
wire messages and persistent formats retain their existing behaviour.

## Validation and provenance

M3 tested fixed fixture `c660491589f959a8ea0d25b580a958cf13110160`;
published head `d7425ec1941d1735af5ede37ea342ec57f1128eb` adds only
documentation. Runtime, dependencies, existing gates and release inputs
match merged M2 `b1fc389`. M4 changes seven Markdown files only.

| Required control | Retained M3 result |
|---|---|
| Rust suite, formatting, Clippy, locked release build | 485 tests; all actual exits 0 |
| Workspace / standalone Quinn-proof audits | PASS; recorded RustSec snapshot `233155b4025b854c68a4b199c0e32d7fd53291a2` |
| Stage12 TLS / QUIC proofs, debug and release | 8/8 and 3/3 each; UDP environment PASS |
| Two-Fang shares / provider and activation controls | 12/12 and 15/15, three rounds |
| Three siblings, late activation, common relay/destination, denial storms | 39/39, three rounds |
| Stage52 resolver faults / hostname migration | 39/39 and 21/21 |
| Stage46 addresses / Stage49 refusal and silent controls | 40/40 and 31/31 |
| Descriptor/quota pressure / Stage43 idle and source fairness | 12/12 and 6/6 |
| S2/S3/S4/S7 P0 / full local CI | 12/12 and 13/13 |

All 26 campaign workers recorded actual exit 0 and passed separate
scenario verifiers. M3 checked 312 source-input hashes and retained
commands, worker identities, actual exits and source/binary provenance
under `target/stage53-m3/`. These ignored local evidence paths are not
committed release artifacts. Full CI is
`target/ci-local/20261117T232918Z/summary.tsv`, including 68 Python tests
and audit; P0 is `target/stage32-chaos/results/20261117T232142Z-network/`.

All 39 membership cases returned initiator descriptors to 16 → 16 and
receiver active/establishing authority to zero. All 3,600 storm clients
reset locally with zero target payload growth and one log per 600-client
burst; 117 new independent-peer clients echoed. Two-second idle CPU
samples were 0.00–0.49%. Provider controls produced 384 committed target
resets with zero extra fallback payload streams, retained existing
fallback sessions and recovered primary capacity.

The orchestration server restarted during M3, but the original manager
and worker survived. Process identities and source hashes were checked;
no duplicate campaign was started. The native manager session later
became unavailable: completion is established by actual worker exits
and the final verifier, not an inferred native manager exit status.
The development smoke's descriptor drain criterion was strengthened to
the original baseline before the final campaign. M2's earlier unchanged
resolver timing failure and successful rechecks remain in its record;
the fresh M3 suite passed without relaxing assertions.

M3 publication passed the installed pre-push Python/audit checks at
`target/ci-local/20261117T233728Z`. All 11 hosted jobs on `d7425ec` were
prevented from starting by GitHub's account billing lock, confirmed
individually from annotations: [push Rust CI](https://github.com/apocsantos/werewolfproxy/actions/runs/44201012218),
[PR Rust CI](https://github.com/apocsantos/werewolfproxy/actions/runs/44201344962)
and [CodeQL](https://github.com/apocsantos/werewolfproxy/actions/runs/44201340668).
No hosted passing result is claimed.

### M4 documentation validation

Full disposable local CI: **13/13 PASS**, all recorded gate exits 0, at
`target/ci-local/20261118T122041Z/summary.tsv`, including 68 Python tests
and audit. All seven document hashes remained unchanged throughout the
suite; this final validation note and wording corrections were completed
afterward. The original completion records were recovered after an
orchestration server restart. A recovery guard stopped before launching
any duplicate gate; no missing native orchestrator exit is inferred.

All **312** recorded M3 source-input hashes still match, all **203** local
document links checked resolve, and whitespace checks pass. Runtime,
dependencies, existing tests/gates, tools, workflows and release inputs
match merged M3 `d3758e9`. Evidence is under `target/stage53-m4/`.
M4 changes seven Markdown files only. The results above are retained M3
evidence, not new M4 Rust/proof/chaos runs.

## Remaining limitations and disposition

- Reserved unused shares reduce utilisation, including memberships at
  unused fallback providers. Prepared/retiring work counts toward the
  64-member bound; an operator may need to close Fangs and await cleanup
  before opening another.
- Late registration does not reclaim active sessions. There is no queue,
  active preemption or bounded admission while old sessions occupy the
  receiver budget. Local release does not prove remote release.
- Existing idle eviction is triggered at the local Fang ceiling or an
  applicable per-source ceiling. A peer share of 32 or 21 exhausted on
  its own does not invoke it. This follows code ordering; M3's Stage43
  regression covers one 64-slot Fang, not smaller-share idle reclamation.
- Coordination covers one daemon's authenticated Fangs, not local users,
  independent daemons using the same key, RPCs or remote pressure. The
  budget assumes the current receiver convention and is not negotiated.
  Shared relay/destination identities can constrain otherwise idle paths.
- Active floods within a Fang can still deny other clients. M3's
  TCP/plain-TCP quota controls denied new clients at 64 active slots,
  then recovered after release. Plain TCP has no peer-share coordinator.
- Fixtures use disposable local identities and loopback/private namespace
  labs. Resolver controls simulate libc errors/blocked work and hosts-file
  changes, not authoritative DNS/TTL or WAN migration. Short CPU samples
  are not comparative performance evidence. Post-ACK target counters
  measure payload-bearing streams, not every empty handshake.

The [current limitations](../KNOWN_LIMITATIONS.md), [status](STATUS.md)
and [index](INDEX.md) now distinguish the implemented policy from the
historical first-come allocation. M1–M3 deliver the approved change and
its local regression evidence; M4 is the closure handoff for review.
No further runtime work is required by the approved Stage53 scope.
Per-user fairness, lending, queuing or smaller-share idle eviction would
need a separate design decision and validation.

Frozen rc.1/rc.2 tags and archives, S3-P2/S2-P4 exclusions, Stage38 closure,
Tokio 1.52.3 and advisory exceptions are unchanged. No installed node,
real Den/peer, service unit or WAN host was used; generated fixture
identities were removed. This is local functional validation, not
universal fairness, comparative liveness or new release certification.
