# Stage46 tools

`dial.py` runs the Stage46 scenarios: a Pack hostname that resolves to
several addresses, dialled by every path (QUIC, encrypted-TCP, plain-TCP,
failover and routed Fangs; discovery and governance sync over QUIC and
TCP). See `docs/STAGE46_M1_DIAL_REPRODUCTION.md`.

- It builds on the Stage45 lab (`tools/stage45/visibility.py`, which uses
  `tests/integration` `ServiceGate`): fresh Dens, the checkout build, only
  its own process groups stopped, and the Stage42 50 MB log guard.
- It always runs in an unprivileged user, mount and network namespace with
  a private root and a private 512 MB `/tmp` (as `tools/stage44/ipv6.py`).
  Inside it, a private `/etc/hosts` defines four names, a private
  `/etc/gai.conf` fixes the order `getaddrinfo` returns their addresses in,
  and a `dummy` interface takes a route to `10.46.0.0/24` and silently
  drops it. The tool checks the order before running anything. The host's
  files and interfaces are not touched.

| Name | Addresses, in order |
|---|---|
| `fast.stage46` | `127.0.0.2` (nothing listens), `127.0.0.1` |
| `silent.stage46` | `10.46.0.9` (dropped), `127.0.0.1` |
| `family.stage46` | `::1` (nothing listens), `127.0.0.1` |
| `dead.stage46` | `127.0.0.2`, `10.46.0.9` |

```sh
python3 tools/stage46/dial.py quic:family tcp:silent discovery-quic:fast \
  --out target/stage46-m1/example.json
```
