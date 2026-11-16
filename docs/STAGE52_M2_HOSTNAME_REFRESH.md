# Stage52 M2 — bounded QUIC hostname refresh

**Status: M2 candidate implemented and locally validated; for review.** The owner
merged [the design](STAGE52_QUIC_DNS_FRESHNESS_SURVEY.md) in PR #162
(`19ea5c2`) and instructed implementation on 2026-11-16. S52-D0–D5 are
applied as recommended. This record covers M2, not Stage52 closure or a
new release.

## Behaviour

Long-lived direct QUIC Fangs, QUIC relay hops and QUIC providers retain
their original `HOST:PORT` as well as their open-time readiness answers.
Each new hostname connection resolves again before racing its addresses.
This also applies to a provider reached through a QUIC relay. Literal
addresses and explicit per-query snapshots avoid refresh, so discovery
and sync retain their current lookup behaviour.

Open-time validation remains outside the daemon state lock, including
current DNS-error codes and initial endpoint bind readiness. Fang relay
preparation is now synchronous: a Pack/route change that invalidates its
preflight answers returns `FANG_ROUTE_INVALID` with a retry message.
It cannot fall into the query resolver while the control path waits for
readiness. Discovery builds query routes separately and resolves once
when executing the query, as before.

Capacity wait, resolution and address racing share the existing **5 s
dial deadline**. The subsequent handshake deadline and failover's outer
**15 s selection deadline** are unchanged. Lookup failures are `dial`
failures; no stale-answer fallback is used. Captured Pack keys, receiver
authorization, provider order and the no-retry-after-ACK rule are unchanged.

Existing QUIC connections are not rebuilt or closed on refresh. Reusable
endpoints stay at at most one per address family. Open-time log addresses
describe readiness answers, not a live DNS cache.

## Resolver capacity and lifetime

New ongoing QUIC refresh work has **16 outstanding jobs per daemon** and
at most one in-flight lookup per reusable peer. Concurrent waiters share
that flight; completed results are released after their waiters leave and
are not reused by later independent attempts. There is no TTL cache or
periodic refresh task.

The asynchronous coordinator obtains a global capacity permit before
submitting blocking system-resolver work. The blocking closure owns that
permit until it actually returns, including if every caller times out or
the Fang closes. A new waiter joins a still-running lookup rather than
starting another. If all waiters leave before submission, the queued job
is cancelled. The coordinator holds only a weak reference to peer state,
so it does not keep a closed Fang alive. Resolver errors/panics return
capacity and allow the next independent lookup; completed-flight cleanup
cannot erase a newer flight.

This bound covers newly introduced ongoing QUIC refresh jobs, not existing
TCP, open-time or discovery resolution. Saturation can time out other
hostname QUIC dials. A libc call can outlive its requester and delay runtime
shutdown; hard cancellation is not claimed. DNS/system caches still apply,
and additional lookups can add latency and observable DNS traffic.

## Validation

Fourteen new Rust tests cover fresh answers and failure/recovery,
cancellation, capacity lifetime after peer drop, coalescing, result
release, closed capacity, wrong-key rejection, address-family changes,
unchanged snapshots/literals and the shared dial deadline. Existing relay
and provider tests also check source retention and preflight validity.

| Check on final runtime/tool source | Result |
|---|---|
| Formatting and all-target/all-feature Clippy, warnings denied | PASS |
| Locked offline workspace/all-feature Rust suite | **474 passed**, actual exit 0 |
| Locked offline workspace/all-feature release build | PASS |
| Stage12 TLS authentication proof, debug/release | **8/8** each |
| Stage12 QUIC authentication proof, debug/release | **3/3** each, UDP environment PASS |
| Full disposable local CI | **13/13 PASS**, actual exit 0 |

The first sandboxed workspace run failed 17 existing filesystem-safety
checks. The host rerun passed all 474 tests without changing code,
assertions or fixture security checks. Both results are retained:
`target/stage52-m2/rust-test{,-host}.{log,result.json}`. Every final check
has its command, input hashes and actual exit recorded by a separate
worker. No Cargo exit is inferred from recovered output.

### Final-source migration matrix

`tools/stage52/refresh.py --expect refresh` ran three fresh labs for each
of seven paths, three clients per phase: **21/21 runs PASS**, actual exit 0.
The tool adds routed failover and explicit baseline/candidate expectations;
historical baseline evidence is preserved. All 63 clients before the
mapping change, all 63 after the mapping change and all 63 after the old
address was withdrawn echoed through their intended primary. All 21 held
sessions echoed across the mapping change before withdrawal. Reopen
controls also echoed, every daemon survived and all 21 cleanups passed.

| Path | Nine clients after withdrawal, without reopen | Seconds per client |
|---|---|---|
| Direct QUIC | All served by B | 0.02–0.04 |
| Routed QUIC hop | All served by C through B | 0.04–0.10 |
| Failover QUIC | All served by primary B; C unused | 0.03–0.05 |
| Failover with QUIC-routed primary | All served by primary C through B; fallback D unused | 0.04–0.09 |
| Direct TCP control | All served by B | 0.01–0.04 |
| Routed TCP control | All served by C through B | 0.09–0.15 |
| Failover TCP control | All served by primary B; C unused | 0.02–0.05 |

Evidence: `target/stage52-m2/migration-final.json` and its recorded worker
exit, on daemon SHA-256
`92ba4d1589a95499fa65bce70195bdfc7165181afb3c9a51e28f912a7b847211`.
An earlier development matrix (`migration.json`) is retained separately;
the table uses only the final-source campaign. No keys were retained.
These are hosts-file/NSS fixtures, not authoritative DNS TTL tests or WAN
migration certification. Existing-session checks occur before the old
network path is removed.

Full local-CI evidence: `target/ci-local/20261116T203741Z/summary.tsv`.
All 68 Python unit tests and the audit passed. No tree edits occurred
during the full suite; this results note was completed afterward. All ten
final-source worker records have identical Rust/tool/manifest input hashes
and actual exit 0. Logs and result metadata are in `target/stage52-m2/`.

## Remaining work and boundaries

M3 still requires the broader resolution-failure/flood and transport/P0
regression campaign from the approved design. M4 limitations and closure
follow that reviewed evidence. M2 passing checks do not close Stage52 or
recertify S3-P2/S2-P4. Stage38 stays CLOSED. Tokio remains 1.52.3;
dependencies, advisory exceptions, wire messages and protocol time/resource
limits are unchanged apart from the approved new resolver-job cap.
Frozen rc.1/rc.2 artifacts are unchanged. No installed node, real Den,
real peer, service unit or WAN host is used.
