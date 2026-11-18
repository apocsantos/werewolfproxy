# Stage53 shared peer-quota reproduction

`peer_quota.py` reuses the Stage42/acceptance lab: checkout builds, dynamic
loopback ports, fresh private Dens and only owned process groups. The
existing 50 MiB per-log guard remains. No installed node or service is
used, and generated identities are removed. There is no keep-temp option.

```sh
python3 -B tools/stage53/peer_quota.py --rounds 3 --active 5 \
  --out target/stage53-m1/matrix.json
```

Optional positional pairs: `quic-quic`, `tcp-tcp`, `quic-tcp`, `tcp-quic`
(default: all). Requires the acceptance lab's Linux/Rust build facilities
and host permissions sufficient for its protected-state filesystem checks.
All targets and Fangs bind loopback. No namespaces or host network changes
are needed. Run sequentially with other lab/CI gates.

Each pair uses receiver A, initiator B with two Fangs to A, and an
independent initiator C. Separate echo targets distinguish the Fangs.
Both B Fangs are opened before load and baseline-tested. C keeps one
established session. In each of two arrival orders:

1. One B Fang opens 64 sessions, each verified by echo. A's local admin
   status must show 65 active, zero establishing sessions (B's 64 + C's 1).
2. The sibling's three new clients must reset, with zero target payload
   streams. Three new clients from C must echo despite B's saturation.
3. Exchange payloads over all 65 held sessions repeatedly for at least
   `--active` seconds, then require three more sibling resets. Existing
   sessions must still echo and A must still report 65 active.
4. Release exactly one B session; require 64 active, then a successful
   sibling session and 65 active again. A new client through the original
   owner must now reset. This identifies a shared slot rather than a
   permanently broken Fang, target or transport.
5. Release B's sessions and require one active session (C's control).
   Both B Fangs must echo without close/reopen. Repeat with roles reversed.

The JSON records per-client outcomes/times, authority snapshots, Fang
health, active-echo rounds, target payload counts, daemon survival,
source/tool/lock/binary hashes and cleanup. Only a reset counts as the
expected denied outcome; timeout, EOF, wrong payload, guard or cleanup
failure makes the tool exit nonzero. Output must be a new file; each
finished pair is checkpointed and failed results are never overwritten.

The default `--expect baseline` retains the original denial assertions.
Expected denial is a confirmed baseline limitation, not a passing fix.
This measures direct authenticated QUIC and encrypted-TCP Fangs with
literal IPv4 peers. Routed/failover contention, new-Fang activation after
saturation, multiple initiator processes with the same key, other peers
sharing a relay, WAN behaviour and policy alternatives are not measured.
Those boundaries are addressed in the [design](../../docs/STAGE53_PEER_QUOTA_FAIRNESS_SURVEY.md).

## Stage53 candidate shares

```sh
python3 -B tools/stage53/peer_quota.py --expect shares --rounds 3 --active 5 \
  --out target/stage53-m2/shares.json
python3 -B tools/stage53/paths.py --out target/stage53-m2/paths.json
```

The shares expectation requires 32 sessions through each already-open
sibling. A full Fang cannot borrow even while the sibling is unused.
After one owner's session closes, its full sibling must still be denied
and the owner must refill that slot. Denials are local `dial` failures.
Both arrival orders, independent-peer admissions, held sessions,
authority snapshots, recovery and cleanup remain checked. Historical
baseline JSON and failed smoke outputs are never reused as candidate runs.

`paths.py` uses the Stage45/46 disposable lab and Stage52 counted/reset
targets, without resolver interposition or a namespace. It checks direct
QUIC/TCP and QUIC/TCP-routed provider paths: local-denial fallback after
32 primary sessions, no payload replay after committed target resets,
and primary recovery. A separate activation case checks 64 registered
Fangs, the 65th open's `FANG_PEER_CAPACITY`, failed-listener rollback and
restored capacity after closing siblings. Optional positional paths:
`quic`, `tcp`, `routed-quic`, `routed-tcp`, `activation` (default: all).
Both tools return nonzero on unexpected outcomes or cleanup failure,
require a new output path and remove generated private fixture state.

These candidate controls do not replace the broader M3 campaign listed
in the [M2 record](../../docs/STAGE53_M2_PEER_QUOTA_ADMISSION.md).

## M3 membership and routed contention

```sh
python3 -B tools/stage53/membership.py --rounds 3 \
  --out target/stage53-m3/membership.json
```

Optional positional cases: `three-quic`, `three-tcp`, `three-mixed`,
`late-quic-quic`, `late-tcp-tcp`, `late-quic-tcp`, `late-tcp-quic`,
`relay-quic`, `relay-tcp`, `destination-quic`, `destination-tcp`,
`storm-quic`, `storm-tcp` (default: all). Each case uses a fresh five-node
disposable lab. It shares the host-permission/build requirements above;
no namespaces, resolver injection or installed-node operations are used.

Three-sibling cases require 22/21/21 → 32/32 → 64 as siblings close.
Late-activation cases preserve 64 existing sessions, demonstrate admission
only after release, then restore 64 to the late Fang after its older
sibling closes. Routed cases isolate common-relay and common-destination
quota keys with authority snapshots and captured path labels; round two
reverses path order. A destination case configures its second route for
later opens while the first Fang keeps its original captured path.

Storm cases use 600 clients at concurrency 16 alongside 32 held sessions,
so they do not exceed the local Fang's 64-slot ceiling. They require 600
local `dial` failures, zero target payload growth and bounded logging.
Every case checks independent held/new clients, idle CPU below 10%, zero
final receiver authority and descriptor drain to its pre-Fang baseline.
Unexpected outcomes, guard or cleanup errors produce a nonzero exit.
Output must be new; completed cases are checkpointed. No private fixture
state or identities are retained. See the [M3 record](../../docs/STAGE53_M3_REGRESSION.md)
for the full campaign and measurement limits.
