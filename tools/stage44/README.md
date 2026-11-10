# Stage44 tools

`ipv6.py` runs the Stage44 IPv6 scenarios (I1–I7). See
`docs/STAGE44_M1_IPV6_REPRODUCTION.md`.

- Every scenario runs in a user, mount and network namespace with a private
  root and a private 512 MB `/tmp`, as `tools/stage43/fairness.py` does.
  Unique-local addresses (`fd43::/64`, plus `fd43:0:0:1::2` in another /64)
  on the namespace's `lo` stand in for IPv6 hosts. Nothing touches the
  host's interfaces, the host `/tmp`, a WAN or any live wolf.
- It reuses the acceptance lab (`tests/integration/run.py`,
  `service_gate.py`) and keeps the Stage42 50 MB log guard. Never remove
  the guard (Stage42 M1 §5).
- The lab's `/tmp` disappears with the namespace, so each result carries
  the daemons' last log lines.
