# Stage53 M3 — peer-session share regression

**M3: PASS on the merged M2 runtime, 2026-11-17.**
The owner merged [PR #167](https://github.com/apocsantos/werewolfproxy/pull/167)
as `b1fc389cab71da277e9c67b098f2f22205c9a225` and instructed continuation.
Application code, dependencies, existing gate definitions and release
inputs remain identical to that merged runtime. The scope follows the
[reviewed design](STAGE53_PEER_QUOTA_FAIRNESS_SURVEY.md) and
[M2 record](STAGE53_M2_PEER_QUOTA_ADMISSION.md).

## Membership and routed-demand fixture

New [membership fixture](../tools/stage53/README.md) reuses the disposable
Stage45/46 lab and Stage52 counted/reset targets. Five generated identities
use dynamic loopback ports: initiator A, receivers/relays B/C/D and an
independent receiver E. The existing 50 MiB per-log guard remains. There
is no resolver injection or namespace in this new fixture, no installed
node and no retained private Den.

Thirteen cases run in three fresh rounds:

- Three siblings over QUIC, TCP or mixed transports must receive
  **22/21/21** reservations. The first cannot borrow the unused siblings'
  shares. Closing the third resets its 21 sessions and permits **32/32**;
  closing the second resets its 32 sessions and permits **64** on the
  remaining Fang.
- Late activation across all four QUIC/TCP pairs must preserve **64**
  already-active sessions. The new Fang is initially denied. Releasing
  one old session lets the late Fang take that slot while the older Fang
  remains above its recalculated share and is denied. Closing the older
  Fang resets its remaining 63 sessions; the late Fang must then hold 64.
- Common-relay cases use two distinct destinations through D over QUIC
  or TCP. Each destination holds 32 while D accounts for 64. The relay's
  shared identity is the binding constraint; each destination alone could
  otherwise receive 64. Closing one sibling restores 64 through the other.
- Common-destination cases open one Fang to B via C, then configure D for
  later opens and open another Fang to B via D. Existing Fangs retain
  their captured route; labels and authority snapshots verify the paths.
  B must account for 64 while each relay accounts for only 32. Thus the
  destination's shared identity is tested independently of relay capacity.
  Arrival order reverses in round two. Closing one sibling restores 64
  through the remaining captured route.
- QUIC/TCP denial storms keep 32 active sessions and an unused sibling's
  reserved share, then submit **600** clients at **16-way** concurrency.
  This stays below the 64-slot local Fang ceiling. Every client must
  reset, the Fang's setup-failure count must increase by exactly 600 as
  `dial`, and no new target payload stream may appear. Logging remains
  bounded by the existing ten-second limiter.

Every case checks established and new independent-peer echoes, daemon
survival, held payloads and final zero active/establishing receiver
authority. All Fangs close and initiator descriptors must return to the
pre-Fang baseline within the existing 15 s fixture readiness window.
Two-second idle CPU observations must remain below 10%; they are not
comparative liveness or throughput measurements. Cleanup stops only owned
process groups and removes every private fixture directory.

The five-case development smoke passed. An initial post-close sample
could observe one descriptor still draining; the fixture was strengthened
from a two-descriptor allowance to waiting for the original baseline.
The subsequent common-destination smoke passed at **16 → 16**. These
development runs remain separate from the final campaign.

Fixed fixture commit: `c660491589f959a8ea0d25b580a958cf13110160`
(following initial fixture `271ae83`).
The final campaign records the exact tool/helper, lockfile, source and
daemon hashes. Evidence root: `target/stage53-m3/`.

### Completed quota-control results

The fixed-source membership matrix passed **39/39**, actual exit 0:
nine three-way cases, twelve late-activation cases, twelve routed cases
and six denial storms. All **3,600** storm clients reset as local `dial`
failures with zero target payload growth; each 600-client burst added one
log line. All **117** independent-peer new clients echoed, as did the held
controls. Every case returned descriptors to its pre-Fang baseline and
all four receiver authority summaries to zero, then passed private-fixture
and owned-process cleanup. Initiator descriptors were **16 → 16** in
every case; two-second idle CPU observations were **0.00–0.49%**. Raw
rows and the separate scenario verifier are retained. The surrounding
campaign also completed successfully, as recorded below.

During resolver testing the orchestration server restarted. The original
campaign manager and resolver worker survived; their recorded process
identities and all source hashes were checked before monitoring resumed.
Completed checks were preserved and no duplicate campaign was started.
The recovery observation is retained in `recovery-observed.json`.

## Final-source campaign results

The independent-worker driver checkpoints commands, process identities,
actual exits and source hashes. It resumes completed work without
inventing a missing exit status. Before and after each gate it checks
fixed source and verifies protected runtime inputs against merged M2.
Legacy harness exit 0 alone is insufficient: scenario outcomes, binaries,
process survival and cleanup are checked separately.

| Control | Result |
|---|---|
| Workspace / standalone Quinn-proof audits | PASS; advisory policy and ignores unchanged |
| Formatting, all-target/all-feature Clippy, locked release build | PASS |
| Locked offline workspace/all-feature Rust suite | **485 passed**, actual exit 0 |
| Stage12 TLS / QUIC proofs, debug and release | **8/8** and **3/3** each; UDP environment PASS |
| Two-Fang shares | **12/12**, three rounds and both arrival orders |
| New membership / route / storm controls | **39/39**, three rounds |
| Provider commitment / activation | **15/15**, three runs of all five controls |
| Stage52 resolver faults / hostname migration | **39/39** and **21/21** |
| Stage46 address matrix | **40/40**, five clients per connection scenario |
| Stage49 refusal/silent/fast controls | **31/31**, three clients; 10,000 one-byte datagrams |
| Stage42/47 descriptor/quota pressure | **12/12**: six at limit 256; QUIC/TCP descriptor cases in three limit-64 rounds |
| Stage43 fairness | **6/6**: idle eviction at 65 s and active-source sharing at 30 s, QUIC/TCP/plain TCP |
| Stage32 S2/S3/S4/S7 P0 | **12/12**, three repetitions per cell |
| Full disposable local CI | **13/13 PASS**, all actual exits 0 |

The Stage43 controls retain their existing thresholds and run once per
transport/scenario as a regression check. The new membership and provider
cases and the two-Fang matrix receive the three-round repetitions above.
Historical Stage43 evidence is not rewritten or recertified by this run.

All **26 campaign checks** recorded actual successful worker exits, then
passed their scenario verifiers. All **312** source-input hashes match
the fixed fixture commit, and the driver/helper hashes remain unchanged.
Protected runtime, test/gate, manifest/lockfile, workflow and release
inputs match merged M2. Evidence index: `target/stage53-m3/verified.json`,
`validation-meta.json`, `summary.tsv` and per-check command/result records.
The campaign began **22:31:48 UTC** and completed **23:35:01 UTC** on
2026-11-17. Worker durations and the server recovery record are retained;
they are not comparative performance or liveness certification.

Full local CI is `target/ci-local/20261117T232918Z/summary.tsv`, including
all **68 Python unit tests** and audit. P0 evidence is
`target/stage32-chaos/results/20261117T232142Z-network/`; every selected
cell has three PASS runs. No tree edits occurred during full CI; this
final results documentation was completed afterward. Observed RustSec
snapshot `233155b4025b854c68a4b199c0e32d7fd53291a2` remained the same at
the final check; workspace and standalone proof audits passed.

Every shares, membership, provider, resolver, migration, address/refusal,
resource and fairness lab row used daemon SHA-256
`7efcf89258c53c48988b5a36062497e54932deb256cc3d8152b6c7538ca26a58`.
CI and P0 retain their separate build-path provenance. All QUIC refusal
paths stayed below **0.1 s**. The receiver discarded **10,000 one-byte
datagrams with zero log lines**, with all six before/after echoes passing.
Across the twelve direct/routed provider runs, all **384** committed
target failures reset clients with **zero** extra fallback payload streams;
existing fallback sessions stayed usable and primaries recovered.

At the 256-descriptor limit, quota controls echoed 62 QUIC clients and
64 TCP/plain-TCP clients. The latter two denied a new client while those
64 active sessions occupied the Fang; all recovered after release and
at the 30 s check. This retains the existing active-flood limitation.
The three limit-64 repetitions peaked at **64/64/64** descriptors for
QUIC and **63/63/64** for TCP, with five-second CPU samples of **1.4–2.0%**
and **2.0%**, respectively. Every case returned to **18** descriptors for
QUIC or **17** for TCP and served legitimate after-release clients.
No log guard fired and no resource/timing criterion was weakened.

Python syntax/CLI help, all **81** local document links checked and
whitespace checks pass. M3 changes only the new fixture and its records;
it does not alter the validated M2 runtime.

## Boundaries and disposition

Q2 still reserves unused shares, including potential fallback memberships.
Late activation cannot reclaim earlier active sessions; admission may
remain denied until those sessions release capacity. Other daemons using
the same key, non-Fang RPCs and remote pressure are outside the local
coordinator. The receiver remains authoritative. Passing controls do not
promise service during saturation or fairness between users within a Fang.

The Stage43 idle regression uses one Fang per receiver, where the local
ceiling and peer budget are both 64. It does not establish eviction at a
smaller reserved share. The unchanged acceptor invokes idle eviction at
the local Fang ceiling or an applicable per-source ceiling; exhausting
a peer share of 32 or 21 alone does not trigger eviction. This boundary
follows the acceptor/admission ordering
in code; it is not a newly measured smaller-share idle scenario.

The resolver fixtures use private hosts mappings and controlled libc
errors/blocking, not authoritative DNS, TTL/cache-expiry or WAN migration.
Post-ACK target counts measure additional payload-bearing streams, not
every empty handshake. No comparative liveness or new release claim is
made. M2's retained resolver deadline failure and later unchanged-source
passing recheck remain documented in the M2 record.

M4 current limitations and closure follow reviewed passing M3 evidence.
Frozen rc.1/rc.2 artifacts, S3-P2/S2-P4 exclusions, Stage38 closure, Tokio
1.52.3, advisory exceptions and protocol/resource bounds are unchanged.
No real peer, installed node, service unit or WAN host is used.
