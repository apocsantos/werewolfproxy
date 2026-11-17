# Stage52 — bounded QUIC hostname refresh closure

**Status: M1–M3 merged; M4 closure for review, 2026-11-17.** The owner
merged the design, implementation and passing regression evidence. This
record completes the approved documentation step on baseline `main`
`3aeb8322c0c222308779e7b425a32a7319ddaada`. Stage52 replaces Stage46's
open-time-only QUIC hostname snapshot for new connections on `main`.
Frozen rc.1/rc.2 tags and archives remain unchanged.

## Delivery and resulting behaviour

| Milestone | Owner-merged delivery | Record |
|---|---|---|
| M1 reproduction and contract | [PR #162](https://github.com/apocsantos/werewolfproxy/pull/162), `19ea5c2`, 2026-11-16 | [Design](STAGE52_QUIC_DNS_FRESHNESS_SURVEY.md) |
| M2 bounded refresh | [PR #163](https://github.com/apocsantos/werewolfproxy/pull/163), `c9e90c8`, 2026-11-16 | [Implementation](STAGE52_M2_HOSTNAME_REFRESH.md) |
| M3 regression | [PR #164](https://github.com/apocsantos/werewolfproxy/pull/164), `3aeb832`, 2026-11-17 | [Results](STAGE52_M3_REGRESSION.md) |

Previously, moving a hostname to a new IP left direct/routed QUIC Fangs
resetting new clients at about 5 s, or failover using a fallback despite
a reachable primary, until reopen. Long-lived direct QUIC peers, QUIC
relay hops and QUIC providers now resolve for each new hostname
connection, including routed failover. Later independent attempts do not
reuse completed answers; concurrent callers may share an in-flight lookup.
Open-time validation remains. Literals and explicit per-query snapshots
retain their existing paths, including discovery/sync resolution.

Capacity wait, resolution and address racing share the existing **5 s
dial deadline**. Subsequent handshake and outer failover deadlines remain.
Missing answers fail the attempt as `dial`, with no stale-answer fallback;
wrong-key answers still fail authentication. Pack identity, target/route
authorization, provider order and the verified-ACK commitment boundary
remain. Existing sessions are not moved or replayed, and reusable QUIC
endpoints remain bounded to one per address family per peer.

New ongoing QUIC refresh work has **16 outstanding jobs per daemon**,
at most one in flight per reusable peer. The blocking call owns capacity
until it actually returns, even after callers time out or a Fang closes.
Unsubmitted work with no remaining waiter is cancelled. This implements
the accepted S52-D0–D5 contract without changing other transport bounds.

## Validation and provenance

M3 tested fixture commit `30367c3dd4f1a631c2e912d19f8ab44ad1e432e9`;
published head `e0a042c91aff9d236aa62f37007c74f56efc2a62` adds the
results documentation. Application source, existing gates, manifests,
lockfiles and release inputs match merged M2 `c9e90c8`. M3's source
hashes also match the merged M3 baseline used for this documentation.
No runtime, dependency, test/gate, workflow or release input changes in M4.

| Required control | M3 result |
|---|---|
| Rust suite; formatting, Clippy and locked release build | 474 tests passed; all checks exit 0 |
| Workspace and standalone Quinn-proof audits | PASS against final RustSec snapshot `233155b4025b854c68a4b199c0e32d7fd53291a2` |
| Stage12 TLS / QUIC proofs, debug and release | 8/8 and 3/3 each; UDP environment PASS |
| Hostname migration, three rounds across seven paths | 21/21 without reopening |
| Resolver failure, wrong key, blocked work, turnover and post-ACK controls | 39/39 across three rounds |
| Stage46 address / Stage49 refusal and silent controls | 40/40 and 31/31 |
| Descriptor/quota controls | 12/12, including three 64-descriptor repeats |
| Stage32 S2/S3/S4/S7 P0 | 12/12 |
| Full disposable local CI | 13/13 PASS, all actual exits 0 |

The [M3 record](STAGE52_M3_REGRESSION.md) indexes commands, source/binary
hashes and scenario outcomes under `target/stage52-m3/`, including
`verified.json` and actual exits for all 20 campaign checks. Full CI is
`target/ci-local/20261117T140224Z/summary.tsv`. These ignored local
artifacts are evidence references, not committed release artifacts.

The initial smoke overloaded a 64-slot Fang with 64 new clients plus a
held control. Its admission reset is retained as a fixture failure; the
corrected 48-client case passed without relaxing the timing bound. The
final campaign spans a long November 16–17 pause, with actual exits and
source hashes preserved. Monotonic worker durations are not presented as
continuous wall-clock performance. Earlier evidence was not rewritten.

Publication of M3 used the installed safe pre-push hook without bypass:
Python unit tests and audit passed at `target/ci-local/20261117T141328Z`.
All **11 hosted jobs** on head `e0a042c` were prevented from starting by
GitHub's account billing lock. Runs: [push Rust CI](https://github.com/apocsantos/werewolfproxy/actions/runs/44100780877),
[PR Rust CI](https://github.com/apocsantos/werewolfproxy/actions/runs/44100899361)
and [CodeQL](https://github.com/apocsantos/werewolfproxy/actions/runs/44100891477).
There is no hosted passing result; billing/workflow settings are unchanged.

### M4 documentation validation

Full disposable local CI on this closure branch: **13/13 PASS**, actual
exit 0, at `target/ci-local/20261117T142019Z/summary.tsv`, including all
68 Python unit tests and the advisory audit. No document edits occurred
during the suite; final documentation was completed afterward. All
**307** recorded M3 source-input hashes still match. All **188** local
document links checked resolve, and whitespace checks pass. M4 changes
seven Markdown files only; the M3 runtime and regression checks above
are retained evidence, not claimed as new M4 runs.

## Remaining limitations and disposition

- Freshness follows the system/NSS resolver and its caches. There is no
  authoritative TTL guarantee, application TTL cache or periodic refresh.
  Additional queries can add latency and visible traffic; resolver failure
  can fail a new session even when an earlier IP remains reachable.
- Blocking libc work can outlive its requester and delay runtime shutdown.
  The 16-job cap bounds newly introduced QUIC refresh work, not existing
  TCP, open-time or discovery lookups. Saturation can time out other
  hostname QUIC dials; literal dials remain independent. Hard cancellation
  and bounded shutdown are not promised.
- Established sessions keep their path and existing liveness bounds.
  Held-session tests cover resolver/mapping changes before withdrawing the
  old network path. Open-time logged IPs are readiness answers.
- Fixtures use Linux/glibc namespaces, private hosts mappings and simulated
  not-found/blocked calls. They do not certify authoritative DNS, cache/TTL
  expiry, real dual-stack networks or WAN migration. CPU samples include
  the interposer's polling. Post-ACK target counters establish zero fallback
  payload streams in the measured runs, not zero empty handshakes.
- Existing active-flood denial remains: TCP/plain-TCP quota controls denied
  a new client while all 64 slots were occupied, then recovered after
  release. Passing regression does not guarantee service during saturation.

The [current limitations](../KNOWN_LIMITATIONS.md), [status](STATUS.md)
and [index](INDEX.md) now distinguish the implemented behaviour from the
historical open-time snapshot. M1–M3 deliver the approved change and its
local regression evidence; this M4 record is the closure handoff for
review. No further runtime work is required by the approved Stage52 scope.

S3-P2/S2-P4 stay EXCLUDED and Stage38 stays CLOSED. The routed-loss
limitation remains. Tokio stays 1.52.3; advisory exceptions are unchanged.
No installed node, real Den/peer, service unit or WAN host was used, and
generated fixture identities were removed. This is local functional
validation, not comparative liveness or a new release certification.
