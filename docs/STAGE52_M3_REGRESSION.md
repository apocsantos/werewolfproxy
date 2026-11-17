# Stage52 M3 — hostname refresh regression

**Delivery note:** the owner merged PR #164 as `3aeb832` on 2026-11-17.
M3 is on `main`; the [M4 closure record](STAGE52_CLOSURE.md) follows.
The campaign results and its original next-step wording below are retained.

**M3: PASS on the merged M2 runtime, 2026-11-16–17.** Baseline is owner-merged
M2 PR #163, `c9e90c8d73a04208a43cc608cee6b2769f84664a`. The runtime and
dependency graph are unchanged for this campaign. The approved scope is
the [Stage52 design](STAGE52_QUIC_DNS_FRESHNESS_SURVEY.md), following the
[M2 implementation](STAGE52_M2_HOSTNAME_REFRESH.md).

## Resolver fault fixtures

[tools/stage52](../tools/stage52/README.md) adds an initiator-only
Linux/glibc interposer for controlled resolver failure and blocked work.
It is loaded only into the disposable A process inside the existing
private user/mount/network namespace. No installed preload configuration,
live node or real identity is used.

The completed three-round matrix has 13 scenarios per round:

- Simulated not-found (`EAI_NONAME`) and real wrong-key IP answers on
  direct, routed, failover and routed-failover QUIC paths. Existing
  sessions must survive the fault; new connections fail or use the
  intended fallback, then recover through the primary without reopen.
- One Fang with 48 new blocked clients plus a held session: only one
  resolver job may start. The load stays below the existing 64-connection
  Fang ceiling. Callers time out, then the Fang closes; the blocking job
  remains alive until explicitly released by the fixture.
- Sixteen blocked jobs across independent Fangs and a seventeenth
  capacity waiter. No seventeenth job starts after its client times out.
  Capacity remains owned after all Fangs close; all 16 slots must then
  refill and complete again. Literal connections work throughout.
- A target reset after commitment must reset the client without replay
  through the fallback. A new connection during the resolver fault
  must be able to use that same fallback, proving it was available.
- Six hundred not-found clients at 48-way concurrency repeatedly drive
  completed-flight cleanup without exceeding the Fang ceiling. Every new
  client resets without a stale target connection, at most one resolver
  job is active per peer, held/literal connections survive, and healthy
  resolution recovers without reopening. Instrumented CPU cost is recorded.

B and wrong-key C use distinct loopback IPs at the same numeric port.
Wrong-key target grants avoid masking identity rejection with policy
denial. The fixture records resolver entry/exit events and checks their
PID, target connection counts, held-session results, descriptor counts,
own-process CPU, source/binary hashes and cleanup. The CPU sample includes
the shim's sleeping polling loop; it is not real-DNS performance evidence.
All clients retain the five-second dial cutoff with 0.5 s scheduling
tolerance. No runtime or existing resource bound is relaxed.

The first development smoke used 64 new clients plus its held control,
exceeding the Fang's 64-slot ceiling; one client correctly reset at
admission. That failure is retained in `target/stage52-m3/smoke.json`.
The fixture was corrected to 48 new clients, preserving the timing bound.
Subsequent coalescing, capacity, refill and post-ACK smoke cases pass.
The separate final campaign passed all 39 scenarios on the checkpointed
fixtures, without changing application source or assertions to pass it.

## Final-source campaign and results

Tested fixture commit: `30367c3dd4f1a631c2e912d19f8ab44ad1e432e9`.
The daemon source, existing tests/gate definitions, manifests, lockfiles
and release inputs are identical to `c9e90c8`. Disposable lab daemon
SHA-256: `92ba4d1589a95499fa65bce70195bdfc7165181afb3c9a51e28f912a7b847211`.
Every resolver, migration, address/refusal and resource case used that
exact binary. CI and chaos retain their own build-path provenance.

| Check | Result |
|---|---|
| Workspace and standalone Quinn-proof advisory audits | PASS; final proof re-audit also exited 0 |
| Formatting, all-target/all-feature Clippy, locked release build | PASS |
| Locked offline workspace/all-feature Rust suite | **474 passed**, actual exit 0 |
| Stage12 TLS proof, debug and release | **8/8** each |
| Stage12 QUIC proof, debug and release | **3/3** each; UDP environment PASS |
| Resolver faults, blocked work, turnover and commitment controls | **39/39**, three rounds |
| Hostname migration | **21/21**, three rounds, no reopen required |
| Stage46 address matrix | **40/40**, five clients per connection scenario |
| Stage49 refusal/silent/fast controls | **31/31**, three clients per connection scenario |
| Stage42/47 resource controls | **12/12**: six at limit 256, six across three limit-64 rounds |
| Stage32 S2/S3/S4/S7 P0 | **12/12**, three per cell |
| Full disposable local CI | **13/13 PASS**, every check exit 0 |

The driver retained actual exits for all **20 campaign checks**, then
verified each scenario's expected outcome, process survival, binary hash
and cleanup. A legacy harness exit 0 alone was not treated as a pass.
Evidence root: `target/stage52-m3/`, including `verified.json`,
`validation-meta.json`, `summary.tsv`, per-check commands and worker
identity/result records. Full CI: `target/ci-local/20261117T140224Z/summary.tsv`.
P0 campaign: `target/stage32-chaos/results/20261117T135448Z-network/`.

The initial advisory snapshot was
`02e2517f457a359fc6d3440ea4560c8ea188fee8`. Full CI refreshed it to
`233155b4025b854c68a4b199c0e32d7fd53291a2` and passed; the proof lockfile
was re-audited against that final snapshot and also exited 0
(`audit-proof-final.result.json`). No ignore entry was added or changed.

### Resolver failures and resources

Missing answers reset direct/routed new clients as `dial`; wrong-key
answers reset them as `handshake`. Failover used the intended fallback
and recovered to the primary without reopening. Existing connections
continued echoing during the resolver faults. The wrong-key answer
changed only the IP, retaining the numeric service port.

| Control, three repetitions | Observation |
|---|---|
| 48 blocked new clients through one Fang | One lookup per run; all 144 clients reset in **5.001–5.035 s** |
| Sixteen jobs plus a capacity waiter | Exactly 16 starts per run; all 51 clients reset in **5.001–5.028 s**, with no late seventeenth job |
| Caller timeout followed by Fang closure | Blocking calls remained active; literal clients still echoed; all 16 slots refilled after completion |
| Descriptor release while blocked calls outlived Fangs | 19–20 descriptors during teardown, returning to **18** after completion |
| Two-second blocked-work CPU sample | **0.0–0.5%** for one job, **3.0–4.0%** for sixteen, including the shim |
| 600 not-found clients at 48-way concurrency | All 1,800 clients reset; 130/138/143 coalesced lookups, maximum one active per peer; **0.15 s** initiator CPU per run |
| Committed target resets on direct/routed failover | All six clients saw RST, zero fallback payload streams observed; a new connection used that same fallback, then recovered to the primary |

Target counters count connections receiving application bytes, not every
raw TCP accept. Thus the post-ACK observations establish no fallback
payload replay in these runs, alongside the unchanged commitment logic
and the Rust/provider regression checks. All 39 daemon-survival and
cleanup checks passed. These measurements are instrumented local
observations, not comparative liveness or real-DNS performance evidence.

### Address/refusal and descriptor/quota controls

All 40 address scenarios matched their expected echo/reset/RPC outcomes.
All QUIC refusal paths in the Stage49 matrix stayed below **0.1 s**.
The receiver discarded **10,000 one-byte datagrams with zero log lines**;
all six before/after clients echoed. Silent paths kept their existing
timeouts, and dead-provider failover still reached the live provider.

At the 256-descriptor limit, descriptor-pressure cases returned to 18
descriptors for QUIC and 17 for encrypted/plain TCP; all legitimate
after-release clients succeeded. The quota cases echoed 54 QUIC clients
and 64 encrypted/plain TCP clients. A new client was denied while the
latter two occupied all 64 slots, as the existing active-flood limitation
allows. All three recovered after release, including the 30 s check.

| Limit-64 control | Flooded descriptors, runs 1/2/3 | Episode CPU seconds | Five-second CPU window | After release |
|---|---|---|---|---|
| QUIC | 64 / 64 / 64 | 0.96 / 0.88 / 0.73 | 1.6 / 1.8 / 1.4% | 18 descriptors, legitimate echo in all runs |
| Encrypted TCP | 63 / 63 / 63 | 1.61 / 1.61 / 1.61 | 2.0 / 1.4 / 1.8% | 17 descriptors, legitimate echo in all runs |

The log guard never triggered. No resource or timing criterion was
weakened, and none of these checks guarantees service during saturation.

## Pause, provenance and disposition

The campaign ran from 2026-11-16 22:28:04 UTC to 2026-11-17 14:08:07 UTC,
spanning a long pause. Independent workers preserved completed checks
and their actual exits; successful checks were not repeated. For example,
`flood64-r2` records start 2026-11-16 22:55:28 UTC, finish 2026-11-17
13:51:45 UTC, exit 0 and **80.8 s of monotonic worker duration**. That is
not a continuous wall-clock performance claim or an attribution of the
pause's cause. Raw records remain unchanged.

All checkpoints used the same committed source and recorded source
hashes. Only this results documentation was completed after the full
suite finished. M3 supplies the required local regression evidence for
the reviewed M2 implementation; **M4 limitations and closure remain next**.

No authoritative DNS/TTL, WAN or new release certification is claimed.
S3-P2/S2-P4 stay EXCLUDED, Stage38 stays CLOSED, and Tokio stays 1.52.3.
Frozen rc.1/rc.2 artifacts, protocol bounds and advisory exceptions are
unchanged. No installed node, real peer or service unit was used. All
generated private identities were removed. Hosted-workflow/billing
settings are outside scope.
