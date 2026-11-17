# Stage53 M2 — reserved peer-session shares

**Delivery note:** the owner merged PR #167 as `b1fc389` on 2026-11-17.
The implementation is on `main`. The [M3 campaign](STAGE53_M3_REGRESSION.md)
tracks subsequent regression; the original candidate results and
remaining-work wording below retain their scope at M2 review.

**Status: M2 candidate implemented and locally validated; for review, 2026-11-17.**
Baseline is owner-merged [PR #166](https://github.com/apocsantos/werewolfproxy/pull/166),
`4fed878649f6c78757a7510892ee15bfb1be9d93`. After that merge the owner
instructed M2 implementation. The [reviewed design](STAGE53_PEER_QUOTA_FAIRNESS_SURVEY.md)
decisions S53-D0–D6 are applied as recommended: Q2 reserved equal shares,
no borrowing and no active-session preemption. This is M2, not Stage53
closure or release certification.

## Resulting behaviour

Every daemon coordinates its authenticated outgoing Fangs by the full
captured receiver public key, shared across QUIC and encrypted TCP.
Names, IPs, bind ports and transport choices do not create new shares.
The receiver's own quota remains authoritative and unchanged.

| Registered Fangs for a receiver | Local reservations available |
|---|---|
| One | 64 |
| Two | 32 each |
| Three | 22 / 21 / 21 |
| 64 | One each |
| 65th registration | Open fails with `FANG_PEER_CAPACITY` |

Remainders follow stable registration order during activation preparation.
Unused shares are reserved, not lent. A new local connection exceeding
its Fang's share is reset promptly; direct/routed setup health records
`dial` with a local-capacity detail through the existing bounded logger.
Failover may try the next provider before verified ACK, in existing order
and within the existing selection deadline. Its final failure class keeps
the existing aggregation across provider failures.

Adding a Fang recalculates shares without ending existing sessions.
An older Fang above its new share cannot admit more until below it;
total local reservations never exceed 64. A Fang opened after another
already holds 64 active sessions can therefore still be denied until
capacity is released. Closing unused Fangs returns their shares after
their owned listener/session work ends.

## Lifetime, activation and routing

`transport/peer_quota.rs` supplies one coordinator owned by `FangRegistry`.
All production open paths pass that coordinator through `FangCancellation`.
The listener registers its immutable path plans before readiness and
activation. The union registration checks every receiver before changing
any membership. Invalid/missing captured public keys fail open with
`FANG_ADMISSION_FAILED`; ordinary bind failures retain `FANG_LISTENER_FAILED`.

Prepared and retiring Fangs remain registered while owned work retains
their membership. Bind/endpoint/activation failure drops the provisional
registration. Listener and child-session references, plus every live
reservation, retain the registration until their work actually ends.
Cancelled/failed attempts return reservations through RAII. Empty receiver
entries are removed, and serials are not reused. No daemon-state lock is
held across a session, resolver call or coordinator wait; reservation
checks use only a short synchronous mutex and never queue.

Direct Fangs reserve their captured peer. Routed Fangs atomically reserve
both relay and destination, deduplicating repeated keys. A failover Fang
registers the union of potential providers and relays, including currently
unused fallbacks. It reserves only the current attempt's direct/routed
set; a pre-ACK failure releases it before the next provider. After verified
ACK the chosen set stays owned for the session, and failures reset rather
than replay through another provider.

Local admission does not prove remote availability: receiver lease release
can lag local closure, another daemon using the same key is outside this
coordinator, and RPC work or remote pressure can still cause refusal.
The fixed local budget assumes the current receiver convention; it is not
negotiated. Unused open/fallback Fangs reduce utilisation. The new 64-member
bound includes registered preparations and retiring work, so an operator
may need to close Fangs and wait for cleanup before another open succeeds.

Plain TCP, discovery/sync queries, payload-copy functions, Pack and target
authorization, provider order, resolver coordination, existing deadlines,
idle eviction, source shares and transport windows retain their paths.
No dependency, wire-message, persistent-state format or hosted workflow
changes are required.

## Unit and disposable controls

Eleven new Rust tests cover capacity/dial classification, stable shares
and remainders, unused capacity, later registration with 64 existing
sessions, full-key grouping and deduplication, atomic routed demand sets,
alternative-plan release, failed registration rollback, invalid identities,
capacity lifetime after member drop, cancellation and repeated cleanup.
A 16-thread test holds concurrent attempts at exactly 32/32 and 64 total.

The extended [two-Fang fixture](../tools/stage53/README.md) preserves its
historical baseline expectation and adds `--expect shares`. Both Fangs
open before load. Each must hold 32 sessions; a full owner cannot borrow
its unused sibling share. With both full, all 65 held sessions (including
the independent peer control) continue echoing. Releasing one owner slot
does not let the full sibling borrow it; the owner refills it. Local
denials must be `dial`. Both arrival orders and cleanup are checked.

`tools/stage53/paths.py` checks direct QUIC/TCP and QUIC/TCP-routed
primaries with a fallback. An unused sibling reserves half the primary's
share: the service's first 32 clients use its primary and the 33rd uses
the fallback after local capacity denial. Resetting all 32 committed
primary targets must produce 32 client resets with zero extra fallback
payload streams; an existing fallback keeps echoing, and primary capacity
recovers. Target counters count payload-bearing streams, not every empty
handshake. The activation control checks the 64-member bound, free port
after failed open, no borrowing from 63 idle memberships, bind-failure
rollback and restoration of 64 sessions after 63 Fangs close.

Development evidence is retained under `target/stage53-m2/`. The mixed
smoke passed. All four provider-path smokes passed; the first activation
smoke failed because the fixture indexed the CLI's error string as an
object. The corrected fixture reads the structured control error and
passed without changing runtime or weakening the assertion. Both runs
remain (`paths-smoke.json`, `activation-smoke.json`). An initial unit
compile error moved public fields out of a drop-protected identity;
the helper was corrected to clone those public fields.

Final fixed-source validation uses commit
`5bd1a0d5b1823e1ee32ca3ebe17fa175af9de456` and checkpointed commands,
input hashes and actual worker exits
under `target/stage53-m2/` and `target/stage53-m2-recheck/`. The first full
workspace run exited 101: 368 of 369 daemon tests passed, including all
11 quota tests and the 10,000-connection QUIC lifecycle case. The existing
`capacity_wait_is_bounded_and_does_not_delay_literal_dials` test timed out
its one-second literal-dial guard at `tests/refresh.rs:106`. It calls
`QuicPeer::dial` directly, without entering the new Fang admission path.
Three isolated exact-test diagnostics passed with the same assertions.
The cause of the original timeout remains unproven; the failed actual
exit and log are retained, not converted into a pass. No runtime/test
source or deadline changed for the separate full recheck, which passed.

| Final fixed-source check | Result |
|---|---|
| Workspace and standalone Quinn-proof audits | PASS; observed final RustSec snapshot `233155b4025b854c68a4b199c0e32d7fd53291a2` |
| Formatting and all-target/all-feature Clippy, warnings denied | PASS |
| Locked offline workspace/all-feature Rust suite | **485 passed**, actual exit 0 |
| Locked offline workspace/all-feature release build | PASS |
| Stage12 TLS / QUIC proofs, debug and release | **8/8** and **3/3** each; UDP environment PASS |
| Unchanged resolver deadline assertion, isolated diagnostics | **3/3**; also passes in the full recheck |
| Two-Fang shares, three rounds across four transport pairs | **12/12**, 24 arrival-order phases |
| Provider and activation controls | **5/5**, including four direct/routed paths |
| Stage52 hostname migration | **21/21**, without reopening |
| Full disposable local CI | **13/13 PASS**, all actual exits 0 |

All **17** recheck workers recorded actual successful exits; each boundary
verified the same **311** source-input hashes. Evidence index:
`target/stage53-m2-recheck/verified.json`, with commands, process identities,
source/tool/driver hashes, timestamps and per-check results. Full CI:
`target/ci-local/20261117T185720Z/summary.tsv`, including all 68 Python
unit tests and audit. No tree edits occurred during full CI; this final
results note was completed afterward. The earlier failed run remains at
`target/stage53-m2/rust-test.{log,result.json}`.

Every shares, provider/activation and migration row used lab daemon SHA-256
`7efcf89258c53c48988b5a36062497e54932deb256cc3d8152b6c7538ca26a58`.
Lockfiles and dependencies are unchanged. All 1,536 owner/sibling fill
sessions echoed, all **288** over-share/no-borrow probes reset, all **72**
independent-peer new clients echoed, and all **24** owner slots refilled.
All **48** after-release probes recovered without reopening. The four
provider paths produced **128** committed resets with **zero** additional
fallback payload streams, and recovered their primary capacity. All
fixture cleanup checks passed and generated private directories were
removed. These are local functional observations, not comparative latency,
throughput or liveness certification.

Python syntax/CLI help, all **79** local document links checked and
whitespace checks pass. This documentation commit does not alter the
validated runtime or fixture source.

## Remaining work and boundaries

M3 still requires the broader pressure/fairness, address/refusal,
resolver-fault, common-relay/common-destination and P0 regression campaign
from the approved design, including three siblings and late activation.
M4 current limitations and closure follow reviewed passing evidence.
M2 does not claim universal fairness, service during saturation or a bound
on admission after late registration. It does not distinguish local users
or an active flood within one Fang.

Frozen rc.1/rc.2 tags and archives stay unchanged. S3-P2/S2-P4 remain
EXCLUDED, Stage38 remains CLOSED and Tokio stays 1.52.3. No installed
node, real Den/peer, service unit or WAN host is used. Disposable identities
are removed, with no keep-temp option in the new fixtures. Comparative
liveness, hosted billing changes and new release certification are outside
scope.
