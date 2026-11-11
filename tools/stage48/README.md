# Stage48 tools

`retry.py` runs the Stage48 scenarios: the encrypted-TCP setup retry
(Stage32 F1) against a peer that does not answer, on the direct Fang and
in failover with one or two silent providers before a live one, with the
QUIC paths as the comparison. See `docs/STAGE48_M1_RETRY_REPRODUCTION.md`.

- It runs in the Stage46 sandbox (`tools/stage46/dial.py`: an
  unprivileged user, mount and network namespace, a private `/etc/hosts`
  and `/etc/gai.conf`, and a `dummy` interface that drops `10.46.0.0/24`)
  and adds two names: `refused.stage46` (`127.0.0.2`, nothing listens) and
  `blackhole.stage46` (`10.46.0.9`, dropped). `dead.stage46` is Stage46's.
- The lab is the Stage46 one with a fourth daemon (D) for the
  three-provider case; the Stage42 50 MB log guard applies.
- Clients wait up to 30 s, so the daemon's 15 s selection window, not the
  client, ends a failover attempt.

```sh
python3 tools/stage48/retry.py tcp:blackhole failover3-tcp:dead \
  --out target/stage48-m1/example.json
```
