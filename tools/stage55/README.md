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
