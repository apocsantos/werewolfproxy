# Stage54 smaller-share idle admission reproduction

`idle_share.py` uses the Stage45/46 checkout lab and Stage53 helpers with
five fresh identities and dynamic loopback ports. Counted targets have no
idle timeout. Only owned process groups are stopped; generated private
Dens are removed. The existing 50 MiB per-log guard applies. There is no
installed-node, service-unit, namespace/resolver change or keep-temp mode.

```sh
python3 -B tools/stage54/idle_share.py --rounds 3 \
  --out target/stage54-m1/matrix.json
```

Optional positional transports: `quic`, `tcp` (default: both). Run
sequentially with other integration/lab gates. Requires the acceptance
lab's Linux/Rust build facilities and host permissions for protected-state
filesystem checks. Output must be a new path; rows are checkpointed
atomically and a failed case stops the matrix without overwriting evidence.

Each round/transport uses one initiator and four receiver identities.
Different receiver keys isolate three local budgets within that lab:

| Receiver | Fangs registered before load | Idle owner | Active sibling controls |
|---|---|---|---|
| B | One | 64 sessions | None |
| C | Two | First Fang, 32 sessions | One |
| D | Three | Second Fang, 21 sessions | Two, one through each sibling |
| E | One | None | One independent-peer session |

All held connections echo before observation. Three extra owner clients
per group must reset before the idle threshold. Then no owner payload is
sent for at least **65 monotonic seconds**. Only the sibling/independent
controls exchange bytes once per second. Every old owner socket must
still be open immediately before the post-idle newcomer.

The 64-slot positive control must admit a newcomer, reach its target and
reset exactly one old idle client. Each smaller owner must still reset
three newcomers locally as `dial`, with zero new target payload streams
and all old clients open. Its receiver has only 33 or 23 sessions, well
below 64. Explicitly releasing one owner connection must permit an echo
without reopening. Remaining owner clients and all active controls echo;
three new independent-peer clients must also succeed.

Finally all Fangs close, receiver active/establishing authority reaches
zero, target handlers drain, initiator descriptors return to the pre-Fang
baseline, all daemons survive and private fixture/process cleanup passes.
Any timeout, unexpected outcome, source change, guard or cleanup failure
returns nonzero. Source/tool/helper/lockfile hashes, checkout commit,
daemon hash, authority snapshots, client timings and outcomes are recorded.

`REPRODUCED` means the documented baseline limitation was measured; it
does not indicate a fix. This matrix covers direct literal-IPv4 QUIC/TCP
Fangs. Routed/failover reclamation, membership changes during reclamation,
non-loopback source limits, WANs and performance comparisons are not
measured. The [policy proposal](../../docs/STAGE54_IDLE_PEER_SHARE_SURVEY.md)
separates these observations from implementation choices requiring review.

## Stage54 candidate reclamation and providers

```sh
python3 -B tools/stage54/idle_share.py --expect reclaimed --rounds 3 \
  --out target/stage54-m2/idle-shares.json
python3 -B tools/stage54/providers.py \
  --out target/stage54-m2/idle-providers.json
```

The original default `--expect baseline` preserves the M1 denial checks.
`--expect reclaimed` instead requires three smaller-share newcomers to
echo after 65 s, each replacing exactly one old idle client. All earlier
pre-threshold, independent/sibling, release-one and cleanup controls remain.

`providers.py` accepts optional positional paths `quic`, `tcp`,
`routed-quic`, `routed-tcp` (default: all). A primary with 32 recently
active sessions must use fallback for its next client. After 65 s with
only fallback/independent controls active, three new clients must reclaim
idle primary sessions, with exactly three old resets and zero additional
fallback payload streams. A primary target fault must reset all 32
committed primary clients without replay, keep the existing fallback
healthy, then recover the primary without reopen. Independent new/held
clients, zero final authority, descriptor and target-handler drain,
owned-process and private-directory cleanup are required.

The same disposable lab and permissions apply. No resolver interposition,
namespace or installed-node operations are used. Output paths must be new;
unexpected results, source changes, guards and cleanup failures return
nonzero and are retained. These M2 controls do not replace the broader
regression listed in the [implementation record](../../docs/STAGE54_M2_IDLE_RECLAMATION.md).
