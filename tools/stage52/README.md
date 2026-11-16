# Stage52 hostname freshness reproduction

`refresh.py` checks baseline snapshots or refreshed hostname connections.
It reuses Stage46's private user, mount and network
namespace, private root/hosts file and disposable Stage45/46 lab.

```sh
python3 -B tools/stage52/refresh.py --rounds 3 --attempts 3 --expect refresh \
  --out target/stage52-m2/migration-final.json
```

Optional positional paths: `quic`, `tcp`, `routed-quic`, `routed-tcp`,
`failover-quic`, `failover-tcp`, `failover-routed-quic` (default: all).
The last path uses C through hostname relay B as primary, then literal D
as fallback. `--expect baseline` (the default) asserts the historical
stale-address behaviour on an unchanged baseline; `--expect refresh`
requires every path to use its intended primary without reopening.
Output must be a new file.
The tool checkpoints each completed scenario and returns nonzero on an
unexpected result. Requires the same Linux namespace/build facilities
as Stage46. No `--keep-temp` option is provided.

Each scenario uses one B receiver process and identity, bound to the
namespace's wildcard IPv4 address. Its private hostname initially maps
to `127.0.0.52`, then `127.0.0.53`. Both addresses initially reach B.
An established session echoes across the mapping change. Then a private
more-specific unreachable route withdraws only the old address. New
clients are measured before/after, followed by closing and reopening the
Fang. No receiver restart, copied Den or forwarding proxy is involved.

Separate counted echo targets identify which failover provider actually
served each batch: B as primary, C as fallback. On the baseline, stale
QUIC skips reachable B until the Fang is reopened; TCP keeps selecting B.
For routing, B is the relay and C is the literal destination. The tool
asserts expected outcomes, provider target counts, daemon survival and
cleanup, and records timings, resolved addresses, binary/lock/tool hashes,
Fang health and the source commit. Generated private lab identities are
removed during cleanup; output contains no private key material.

The original six-path baseline record is preserved in the Stage52 survey;
it predates the additional routed-failover path. This is a deterministic
hosts-file/NSS change, not an authoritative DNS
server test, TTL experiment or WAN migration. The established-session
check occurs before address withdrawal; it makes no survival claim after
the underlying path is removed. These expected baseline failures are
reproduction results, not transport certification. Candidate refresh
results likewise do not establish real DNS TTL or WAN behaviour.

## M3 resolver faults and lifetime controls

```sh
python3 -B tools/stage52/faults.py --rounds 3 --attempts 3 \
  --out target/stage52-m3/resolver-faults.json
```

This adds 13 scenarios per round: missing answers and wrong-key answers on
direct, routed, failover and routed-failover QUIC; 48 coalesced callers;
16-job saturation with a 17th waiting Fang; a 600-client not-found storm
at 48-way concurrency; and direct/routed failover
after a committed session's target fails. Optional positional scenario
names are listed by `--help`. Output must be a new file.

`resolver_shim.c` is a Linux/glibc **test interposer**, compiled with the
local C compiler. Only the disposable initiator A loads it through its
own `LD_PRELOAD` environment. It counts selected `.stage52` lookups and
injects `EAI_NONAME` or blocks them on a private mode file. Literal
lookups and other processes pass through unchanged. Blocking has a 30 s
fixture guard; reaching it fails the test. Log entries are checked for
the expected initiator PID. No preload configuration is installed.

Wrong-key cases use the actual private hosts file: different generated
identities B and C listen at the same numeric port on different loopback
addresses. Changing only the IP therefore reaches the wrong key. Targets
are explicitly granted so a target denial cannot hide a failed identity
check. Already established connections must keep echoing during faults;
new pre-ACK failover must select the fallback and recover without reopen.

The flood checks require timed-out clients to reset within the existing
dial window with 0.5 s scheduling tolerance. Blocking work must outlive
caller timeout and Fang closure; literal connections must still work.
All 16 slots are refilled after completion, then released again, before
healthy recovery. A two-second own-process CPU sample must remain below
10%; it includes the shim's sleeping polling loop and is not a real-DNS
performance benchmark. The existing 50 MB lab log guard remains active.

The interposer's not-found result represents a resolver failure; it is
not an authoritative DNS-server/NXDOMAIN or TTL test. Reports preserve
per-client outcomes, target counts, resolver entry/exit events, CPU and
descriptor observations, source/library/daemon hashes and cleanup status.
All generated identities, sockets and copied libraries are removed.
