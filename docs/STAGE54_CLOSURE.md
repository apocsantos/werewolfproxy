# Stage54 — bounded idle peer-share reclamation closure

**Status: M1–M3 merged; M4 closure for review, 2026-11-21.** The owner
merged the reviewed contract, implementation and regression, then
instructed continuation. This documentation step starts from merged
`main` `4ff2feef7191e5a0f476cc3820aee2bb66b58951`.

## Delivery and resulting behaviour

| Milestone | Owner-merged delivery | Record |
|---|---|---|
| M1 reproduction and I2 contract | [PR #170](https://github.com/apocsantos/werewolfproxy/pull/170), `4dd8dda`, 2026-11-19 | [Design](STAGE54_IDLE_PEER_SHARE_SURVEY.md) |
| M2 bounded reclamation | [PR #171](https://github.com/apocsantos/werewolfproxy/pull/171), `a8bcc12`, 2026-11-20 | [Implementation](STAGE54_M2_IDLE_RECLAMATION.md) |
| M3 broader regression | [PR #172](https://github.com/apocsantos/werewolfproxy/pull/172), `4ff2fee`, 2026-11-21 | [Results](STAGE54_M3_REGRESSION.md) |

Stage53 reserves a daemon-local 64-session budget per captured receiver
key: two Fangs receive 32 slots each; three receive 22/21/21. Previously,
an idle Fang below its local/source ceiling could fill its smaller peer
share indefinitely and deny new clients. Stage54 adds pressure-driven
reclamation to that boundary, as accepted in S54-D0–D6 (I2).

On a typed local peer-capacity denial, the current attempt may replace
one committed session in its own Fang that has seen no successful payload
I/O for at least **60 s**. The candidate must actually hold the blocking
receiver keys, and releasing just that reservation must satisfy every
current share and total check. Longest idle wins, with stable identifier
ties. The selection rechecks current pressure and activity before marking;
setup work, recent sessions, siblings and unrelated paths are excluded.

Cancellation retains real quota until owned cleanup releases it. One
nonblocking reclamation operation per Fang waits at most **1 s**, within
the original dial/selection deadline, then rechecks the complete demand
set once. There is one victim budget per newcomer, shared with earlier
local/source eviction and later provider attempts. A busy gate, slow
cleanup or capacity race may still deny the newcomer.

Direct QUIC/TCP, captured routed demands and failover attempts follow the
same rule. Eligible idle capacity at the current provider is reclaimed
before advancing to a free fallback; active primary saturation retains
fallback behaviour. Only the final destination's verified ACK commits
the attempt. After that, failure resets the client without retry or
payload replay through another provider.

There is no idle expiry timer, borrowing, forced rebalance or multi-victim
eviction. Membership still preserves existing over-share sessions and
unused shares. Receiver authority, the 64-member bound, local/source/global
ceilings, plain-TCP rules, resolver limits, transport windows, wire and
persistent formats remain unchanged. The fix is absent from frozen rc.1
and rc.2 artifacts.

## Retained validation and provenance

M3 ran fixed fixture/source commit
`4b48bb5b3ff2473684ce9e9c2875bd97560c1028`; published head
`0a61c8dca0995af680566b15f11aec10725e220e` adds documentation notes only.
Its **31 workers and overall manager exited 0**, and separate scenario
verifiers passed. M4 retains these results; it does not claim a separate
local Rust/proof/chaos campaign.

| Required coverage | Reviewed M3 result |
|---|---|
| Workspace, audits and authentication | 497 Rust tests; format, Clippy, release build and both advisory audits; debug/release TLS 8/8 and Quinn 3/3 each |
| 64/32/21 and provider preference | 18 groups, 42 exact idle replacements; four direct/routed provider paths, twelve primary replacements, 128 committed resets with zero fallback replay |
| New boundaries | Six labs: 22-slot mixed shares, recent/active protection, grandfathered membership, provider victim budget and half-close |
| Routed contention | Twelve labs, 85 concurrent admissions and 85 victims; 7,200 active-pressure resets, zero target payload growth, all sibling sessions preserved |
| Source accounting | Six private-namespace labs; source/peer victim budgets and six permitted initial denials followed by free-slot recovery without another victim |
| Earlier regression | Stage53 shares/membership/providers 12/39/15; resolver/migration 39/21; address/refusal 40/31; resource/fairness 12/6 |
| P0 and local CI | S2/S3/S4/S7 three trials each, twelve total; 13/13 local gates, including 68 Python tests and audit |

All **34** Stage54 native labs use full 65-second observations, return
initiator descriptors **16 → 16**, drain target handlers and receiver
active/establishing authority, preserve daemon survival, and remove owned
processes/private Dens. New boundary/routed/source labs measured two-second
idle initiator CPU at **0–0.49%**. Half-close release uses original target
handler identities because an already-consumed FIN continues to report
EOF. Membership changes during the release wait and slow-cleanup timing
are covered by deterministic unit tests, not exact native await injection.

M3 evidence is retained under `target/stage54-m3/`: 317 source hashes,
matrix digests, commands, actual worker exits, manager observation and
binary provenance. Full CI is `target/ci-local/20261121T002416Z/`;
P0 is `target/stage32-chaos/results/20261121T001640Z-network/`. Native
debug daemon SHA-256:
`6183b3155cb5613b8be193dbbebac2a7a425f713477748b72ddffbe86870f43b`.
Lockfile SHA-256:
`411ef3b496dfd8c21f97eb2285d55bd0d12ae604436acfdfa16044e80a0d91d0`.
The recorded RustSec snapshot is `f291af9b1cc0d21dfcea1709d6b037963cef262b`.
Ignored local evidence is not a release artifact or a retained identity.

Four fixture-development invocations exited 1 and cleaned up; their
separate smoke index retains setup/observation errors and the subsequent
passing source-TCP smoke. The final frozen M3 campaign passed without
retrying a failed worker or relaxing existing gates. The M2 record also
retains the earlier disappearing-pressure red/green regression.

M3's installed pre-push checks passed at
`target/ci-local/20261121T003234Z/`. All **12 hosted checks** on `0a61c8d`
passed, including push/PR Rust CI and CodeQL's **No new alerts in code
changed by this pull request**, observed 2026-11-21 00:40:08 UTC. These
are fresh M3 results; earlier stages' billing-blocked hosted records
remain historical. [PR #172](https://github.com/apocsantos/werewolfproxy/pull/172)
retains the publication handoff.

## M4 documentation validation

Full disposable local CI: **13/13 PASS**, every recorded gate and the
overall manager exit **0**, at
`target/ci-local/20261121T204235Z/summary.tsv` (**342.7 s**), including
all **68** Python unit tests and the fresh workspace advisory audit.
All eight document hashes stayed fixed throughout the suite; this result
note was added afterward. No separate local M4 Rust/proof/chaos campaign is claimed.

All **317** recorded M3 source-input hashes still match, all **228** local
document links resolve and whitespace checks pass. Runtime, dependencies,
existing tests/gates, tools, workflows and release inputs match merged M3
`4ff2fee`. M4 changes eight Markdown files only. Evidence under
`target/stage54-m4/` retains the fixed document hashes, source checks,
actual worker/manager exits and final verification.

## Remaining limitations and disposition

- A legitimate idle SSH or database session may reset under peer pressure
  even below the local 64-slot ceiling. Transport keep-alives do not count
  as application payload activity. Activity sampling/rechecking does not
  provide an application-level idle guarantee against concurrent bytes.
- One useful victim does not guarantee admission: cleanup, membership or
  competing capacity can race the final acquisition. A source eviction
  may spend the budget before peer quota is released, producing a reset
  followed by recovery in the free slot. No second victim or queue is used.
- Late activation never forces old sessions down to a smaller share. If
  one release cannot fit the attempt, peer reclamation chooses no victim.
  Unused provider/route memberships still reserve shares and count toward
  the 64-member bound until their owned work ends.
- Active saturation still denies clients within a Fang. Coordination is
  per daemon and authenticated receiver key, not per user, independent
  process, RPC or remote receiver workload. Local release does not prove
  remote availability; receiver quotas and authorization still apply.
- Evidence uses disposable local identities, loopback/private namespaces
  and short idle CPU samples. It does not certify WAN behaviour, suspend,
  authoritative DNS/TTL migration, throughput or comparative liveness.
  Post-ACK target counts measure payload-bearing streams, not every empty
  handshake. Existing half-close transport differences remain.

The [current limitations](../KNOWN_LIMITATIONS.md), [status](STATUS.md)
and [transport overview](../TRANSPORT_ARCHITECTURE.md) now describe the
implemented policy and its compatibility cost. M1–M3 deliver the approved
runtime and regression; M4 is the documentation closure handoff for review.
No further runtime work is required by the approved Stage54 scope.

Frozen rc.1/rc.2 artifacts, S3-P2/S2-P4 exclusions, Stage38 closure,
Tokio 1.52.3 and advisory exceptions remain unchanged. No real Den/peer,
installed node, service unit or WAN host is used.
