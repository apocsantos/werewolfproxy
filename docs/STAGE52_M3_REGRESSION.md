# Stage52 M3 — hostname refresh regression

**Status: campaign prepared; validation pending.** Baseline is owner-merged
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

The planned three-round matrix has 12 scenarios per round:

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
Final campaign results will be recorded separately after validation.

## Remaining gates

The campaign repeats migration, Stage46 address controls, Stage49
refused/silent and datagram controls, Stage42 descriptor/quota pressure
and Stage47 CPU controls. Required Stage32 cells are S2/S3/S4/S7 P0,
three runs each. Rust checks, Stage12 authentication proofs and full
disposable local CI remain required. Each check must retain its actual
exit and verify scenario outcomes, rather than treating a harness exit
alone as a verdict.

No authoritative DNS/TTL, WAN or new release certification is claimed.
S3-P2/S2-P4 stay EXCLUDED, Stage38 stays CLOSED, and Tokio stays 1.52.3.
Frozen rc.1/rc.2 artifacts, protocol bounds and advisory exceptions are
unchanged. M4 limitations and closure follow the reviewed M3 evidence.
