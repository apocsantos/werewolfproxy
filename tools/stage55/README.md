# Stage55 daemon-wide idle admission reproduction

```sh
python3 -B tools/stage55/global_idle.py --rounds 3 \
  --out target/stage55-m1/matrix.json
```

Checkout-only two-node Stage45/46 lab, generated identities, dynamic
loopback ports and the existing 50 MiB per-log guard. Only owned processes
are stopped and private fixture directories are removed. Requires Linux
and the existing protected-filesystem lab permissions. No installed node,
real peer, namespace change or keep-temp mode is used.

The fixture and its children set their own descriptor limit to **4096**;
the invoking shell and other processes are unaffected. Thirty plain-TCP
Fangs hold 32 echoed sessions each and one holds 64, filling the real
production **1024** global slots. A separate empty Fang is a control.
One buffered selector-based echo loop handles the target sockets without
creating 1024 target threads. The fixture does not lower a runtime cap.

All 1024 clients first echo and remain open without payload for at least
65 monotonic seconds. Before the wait, probes through the 32- and 64-slot
Fangs must reset. Afterward:

- Three newcomers through the 32-session Fang and three through the empty
  Fang must reset; every old client must stay open and target payload
  streams must not grow.
- A newcomer through the 64-session Fang must echo, reset exactly one old
  client in that Fang, and preserve the other Fangs' 960 clients. The global
  cap remains full, so the 32-session Fang must still deny its next probe.
- Releasing one different Fang's session must drain two initiator
  descriptors and recover admission through the 32-session Fang without
  another victim. All 1024 surviving sessions must still echo.

Final targets and initiator descriptors drain, daemons survive, two-second
idle initiator CPU stays below 10%, and owned-process/private-directory
cleanup must pass. Output paths must be new. Source hashes and phases/rows
checkpoint atomically; an unexpected outcome stops with failure evidence.
Defaults to three fresh labs. `REPRODUCED` is the expected baseline denial,
not a passing implementation of a new policy.

This native matrix measures **plain TCP only**, isolating the shared
acceptor from authenticated receiver quotas. Source inspection establishes
that secure paths also use this global gate before session setup; their
full-cap native behaviour and any proposed fix still require separate
validation. See the [survey](../../docs/STAGE55_GLOBAL_IDLE_ADMISSION_SURVEY.md)
for evidence levels, policy choices and scope.

## Stage55 M2 secure/plain admission controls

```sh
python3 -B tools/stage55/admission.py quic tcp --expect baseline --rounds 1 \
  --out target/stage55-m2-baseline/secure.json
python3 -B tools/stage55/admission.py --expect reclaimed --rounds 3 \
  --out target/stage55-m2/global.json
python3 -B tools/stage55/admission.py quic tcp --expect reclaimed --rounds 1 \
  --routed-owner --out target/stage55-m3/routed.json
python3 -B tools/stage55/admission.py --expect reclaimed --rounds 1 \
  --concurrent-probes 12 --out target/stage55-m3/concurrent.json
```

The baseline invocation is for the unfixed runtime. The original M1
`global_idle.py` remains unchanged and retains its old denial assertions.
`admission.py` defaults to QUIC, encrypted TCP and plain TCP; optional
positional transports select a subset. Output paths must be new.

Each secure lab uses one 32-session secure owner and 992 plain fillers,
reaching production global capacity while receiver authority is only 32.
After 65 s, baseline mode requires three resets and zero old victims;
reclaimed mode requires three echoes and exactly three own idle victims.
All 992 sibling sessions survive. Empty-Fang probes still reset, and
refreshing all owner's sessions protects them from further reclamation.
The full-local 64 positive control and different-Fang release recovery
remain, with 1024 surviving echoes, bounded idle CPU, zero final receiver
authority, target/descriptor drain and private cleanup.

Same loopback-only lab, 4096 task-owned descriptor bound, 50 MiB log guard,
atomic phase/row checkpoints and source hashes as M1. No native peer quota
or global limit is reduced. See the [M2 record](../../docs/STAGE55_M2_GLOBAL_RECLAMATION.md)
for the shared victim budget and setup deadline accounting.

M3's `--routed-owner` control uses a generated relay and holds the secure
owner path through the same 1024 global slots; its 992 plain siblings remain
independent. `--concurrent-probes N` starts N owner demands together after
the idle interval, retains successful sessions, and verifies the corresponding
number of owner victims while siblings survive. Both controls preserve the
empty-Fang, recent-owner, local-capacity, release-recovery, echo, descriptor,
authority and cleanup checks. The checkpointed campaign also retains the
Stage54 source, route/provider, membership, resolver, resource, fairness and
chaos matrices plus full local CI. See the [M3 regression record](../../docs/STAGE55_M3_REGRESSION.md).
