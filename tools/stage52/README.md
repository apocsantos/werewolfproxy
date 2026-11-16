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
