# Stage45 tools

`visibility.py` runs the Stage45 scenarios: Fang sessions that fail, and
what the initiator logs and lists. See
`docs/STAGE45_M1_VISIBILITY_REPRODUCTION.md`.

- It builds on the acceptance lab (`tests/integration/run.py`,
  `service_gate.py`): fresh Dens, dynamic loopback ports, only its own
  process groups stopped, no live wolf touched.
- It keeps the Stage42 50 MB log guard. Never remove it: `flood:tcp-plain`
  exists to measure log growth (Stage42 M1 §5).

## A hostname that resolves to IPv4 only

The lab's wolves listen on `127.0.0.1`. On Debian `localhost` resolves to
`::1` first, and a QUIC Fang dials the first address (S44-D4), so
`hostname:quic` with the default name gets no answer. `--sandbox-hosts`
runs the lab in an unprivileged user, mount and network namespace (as
`tools/stage44/ipv6.py`: private root, private 512 MB `/tmp`) whose
private `/etc/hosts` maps `--hostname` to `127.0.0.1` only. The host's
files and interfaces are not touched.

```sh
python3 tools/stage45/visibility.py hostname:quic hostname:tcp \
  --hostname wolf-b.stage45 --sandbox-hosts \
  --out target/stage45-m2/m2-hostname4.json
```
