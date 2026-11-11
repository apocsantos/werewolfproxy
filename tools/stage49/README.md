# Stage49 tools

`refused.py` runs the Stage49 scenarios: a QUIC peer whose host is up but
nothing listens on its port, on every dial path (direct, failover with
one or two such providers, routed, discovery, sync), with a silent peer
and a name whose first address refuses as controls, and the TCP paths as
the comparison. Its `datagrams` scenario sends 1-byte UDP datagrams to a
live QUIC receiver and records whether it logs, spends CPU or stops
serving. See `docs/STAGE49_M1_QUIC_REFUSED_REPRODUCTION.md`.

- It runs in the Stage46 sandbox (`tools/stage46/dial.py`) with the
  Stage48 lab and names (`tools/stage48/retry.py`): `refused.stage46`
  (`127.0.0.2`, nothing listens),
  `blackhole.stage46` (`10.46.0.9`, dropped) and Stage46's
  `fast.stage46` (`127.0.0.2`, then `127.0.0.1`).
- The Stage42 50 MB log guard applies; the host is not touched.

```sh
python3 tools/stage49/refused.py quic:refused sync-quic:refused datagrams \
  --out target/stage49-m1/example.json
```
