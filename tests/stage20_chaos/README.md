# Stage 20 adversarial soak harness

`soak.py` is a disposable Linux loopback campaign for the released
`werewolfd` and `werewolfctl` binaries.  It creates two fresh Dens and two
local target servers, exchanges only generated public Pelt material through
the supported CLI, and removes the entire fixture at exit.

The default `ci` profile is intentionally short.  It exercises reconnects,
mixed transports, restart/kill recovery, Silver and revocation fencing,
manifest-generation churn, malformed TCP/UDP input, bounded pre-auth and
authority saturation, control-plane churn, large transfers, and resource
sampling.  It is suitable for a normal regression run:

```sh
python3 -B tests/stage20_chaos/soak.py \
  --daemon /path/to/werewolfd --control /path/to/werewolfctl \
  --tmp-parent /secure/private/tmp
```

The `extended` profile performs 1,000 authenticated cycles for each secure
transport, 100 peer restarts and SIGKILL recoveries, and ten 50 MiB transfers
per transport.  `--min-duration-seconds` keeps a deterministic, seeded
alternating secure-forwarding loop running after those mandatory checks.  The
script prints one sanitized JSON result; it writes no payloads, packet
captures, keys, or persistent reports.

```sh
python3 -B tests/stage20_chaos/soak.py \
  --profile extended --seed 20260916 --min-duration-seconds 7200 \
  --daemon /path/to/werewolfd --control /path/to/werewolfctl \
  --tmp-parent /secure/private/tmp
```

The harness does not manipulate host routes, wall clocks, disk capacity, or
systemd.  Those boundaries are covered by the existing Stage 15B–19 fault,
lifecycle, and installed-artifact suites, and are recorded separately in the
Stage 20 certification document.
