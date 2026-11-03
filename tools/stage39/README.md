# Stage39 M1: relay-to-destination liveness reproduction (diagnostic only)

These scripts are test-only. They run the real Stage26 routed Fang
(A → Y → D) inside the Stage32 sandbox (`tests/integration/chaos.py`,
which uses unprivileged user, network and PID namespaces), against
unchanged production binaries. They do not change production code, any
Stage32 rule, profile or verdict, or the host's network or sysctls.

The record is `docs/STAGE39_M1_LIVENESS_REPRODUCTION.md`.

| File | Role |
|---|---|
| `relay_target_liveness.py` | builds the sandbox and the routed Fang, then for each scenario: injects the Y ↔ D fault, samples once per second, closes the client, and samples again |
| `s39_targets.py` | D's test services on 127.0.0.1: `echo`, `sink` and `source` |
| `s39_client.py` | A's application client. It never closes until told to; it logs EOF, reset or errors against monotonic time. |
| `analyse.py` | summarises the records; times are relative to fault injection |

The scenario format is `HOP:FAULT:ACTIVITY:REP`:

- **HOP:** `tcp` or `quic`, the outer A → Y hop.
- **FAULT:**
  - `none`: no fault;
  - `blackhole`: netem loss 100% on both ends of D's link;
  - `linkdown`: the router-side end of D's link set down, which causes
    routing errors.
- **ACTIVITY:** `idle`, `upload` or `download`.

The protocol is declared in the script header, before any run: fault at
5 s, a 300 s window, a 150 s bound, then 90 s after the client closes.

## Reproduce

```sh
cargo build --release --locked --offline --workspace --bins --target-dir target/stage39-m1/build
python3 -B tools/stage39/relay_target_liveness.py target/stage39-m1/build/release \
    target/stage39-m1/runs $(cat target/stage39-m1/campaign.scenarios)
python3 -B tools/stage39/analyse.py target/stage39-m1/runs
```

`S39_SMOKE=1` shortens the window, as a topology check only. A smoke run
is never evidence.

## M3: D07 target-failure equivalents (sandbox)

`d07_sandbox.py` runs Stage22R D07's target behaviours (`d07_target.py` on
D: `echo`, `rst`, `kill`, `fin`, `absent`) through a Fang from
`d07_client.py` on A, with a 64 MiB upload and the target acting after
8 MiB. It uses D07's series and its pass rules, which are fixed in the
script header. Paths: `direct-tcp`, `direct-quic`, `routed-tcp` and
`routed-quic`. After every run, the leases must clear within 10 s; after
a settle, the Stage32 leak bounds apply.

```sh
python3 -B tools/stage39/d07_sandbox.py BIN_DIR target/stage39-m3/d07 \
    direct-tcp direct-quic routed-tcp routed-quic
```
