# Stage44 M2 — IPv6 fix

**M2: PASS.** Every M1 scenario passes on the M2 build. That covers:

- QUIC over IPv6, wherever it failed in M1: the QUIC Fang, the routed
  QUIC hop, discovery, sync, and an IPv6 peer of a dual-stack receiver.
- One IPv6 /64 is held to 16 slots of a Fang.
- `--bind ::1` is recorded as `[::1]:PORT`.

The IPv4 path is unchanged, and every existing check passes.

## 1. Change (`01d0457`)

| Option | Where | Change |
|---|---|---|
| V1 (S44-D1) | `quic_fang.rs` `client_bind`, `client_endpoint` | a QUIC client endpoint dialling an IPv6 destination binds `[::]:0` (dual-stack where the host allows). Every other case keeps `0.0.0.0:0` exactly as before, including a destination that is not a literal socket address. So a host with IPv6 disabled loses nothing |
| | callers | the QUIC Fang (`quic_server` parsed once at start), `connect_v3`, discovery and sync RPCs (`discovery.rs`), the routed forwarder and `over_relay` (`relay.rs`, relay address), failover (`failover.rs`, the QUIC provider's address or its relay's) |
| V2 (S44-D2) | `fang_accept.rs` `share_key` | a non-loopback IPv6 source is counted under its /64. IPv4 and IPv4-mapped sources keep their exact address, and loopback stays exempt |
| V3 (S44-D3, kept by the owner as a spelling cleanup) | `werewolfctl` `local_listener` | an unbracketed IPv6 `--bind` is bracketed (`::1` → `[::1]:PORT`). Any other value passes through unchanged, as before |

V3 is narrower than the survey's wording ("a hostname is refused at the
CLI"). M1 showed `--bind ::1` already worked. Refusing hostnames would
have narrowed the documented `--bind ADDR` (`docs/V1_CLI_CONTRACT.md`) for
no defect, so M2 does not.

No protocol, trust, TLS, window, timeout, state-format or dependency
change. The silent QUIC failure and the `rejected` status (M1 §4) lose
their cause but are otherwise unchanged.

## 2. Unit tests (4 new)

| Test | Shows |
|---|---|
| `quic_fang::tests::client_endpoint_family_follows_the_destination` | IPv4 destination and none → `0.0.0.0:0`; IPv6 and IPv4-mapped IPv6 → `[::]:0` |
| `quic_fang::tests::quic_reaches_an_ipv6_receiver` | the IPv4 endpoint refuses an IPv6 receiver with `InvalidRemoteAddress` (the M1 cause); the V1 endpoint completes the handshake to `[::1]` and carries a stream |
| `fang_accept::tests::ipv6_sources_share_by_slash_64` | addresses in one /64 share a key; another /64 does not; IPv4 and mapped keep the exact address; `::1` exempt |
| `werewolfctl` `service_connect_brackets_an_ipv6_bind` | `::1`, `[::1]` and `fd43::2` give bracketed listeners; IPv4 binds unchanged |

## 3. Lab on the M2 build (one run each)

Lab build `werewolfd`
`a5d259d32695dd919f05b9edfd0a25d4de273296b0ea435648f4a3082dea4a96`.
`tools/stage44/ipv6.py`, same sandbox and scenarios as M1. Results are in
`target/stage44-m2/m2-ipv6.json`.

| Scenario | M1 (`main`) | M2 |
|---|---|---|
| I1 `fang:quic` | echo reset; 1 MiB 0 bytes | echo PASS; 1 MiB PASS (SHA-256) |
| I2 `fang:tcp`, `fang:tcp-plain` | PASS | PASS |
| I3 `discovery:quic` | `rejected`, nothing cached | `ok`, 1 record cached |
| I3 `discovery:tcp` | `ok` | `ok` |
| I3 `sync:quic` | `rejected`, head 0 | `ok`, applied [1], head 1 |
| I3 `sync:tcp` | applied [1] | applied [1] |
| I4 `routed:quic` | echo reset | echo PASS; 1 MiB PASS |
| I4 `routed:tcp` | PASS | PASS |
| I5 `dualstack` | IPv6 QUIC reset; others PASS | all four PASS |
| I6 `share:tcp`, `share:tcp-plain` | 16 × 4 = 64 held; other /64 reset | first address 16, the other three **0**; other /64 PASS during the flood; PASS after release |
| I7 `cli` | `::1:P` / `[::1]:P` | `[::1]:P` / `[::1]:P` |

The daemons stayed up in every scenario, the log guard never triggered,
and host `/tmp` stayed at 5%.

## 4. Existing checks

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --workspace --all-targets --all-features -D warnings`:
  PASS.
- `cargo test --workspace --all-features`: **428 passed**, 0 failed (424
  plus the 4 new tests).
- `scripts/ci-local.sh` full (`target/ci-local/20261110T163443Z`): **13
  of 13 PASS**. This includes acceptance (all transports over IPv4, a
  50 MiB transfer), the routed-Fang, discovery, sync and provider gates.

## 5. Next

M3 (§6 of the survey):

- the M1 IPv6 scenarios ×3;
- the Stage43 idle and source scenarios and the Stage42 flood scenarios,
  IPv4, one round each;
- the Stage32 cells S2 P0/P1, S3 P0, S4 P0 and S7 P0, because the QUIC
  dial path changed;
- the routed-Fang gate.

Every historical verdict must hold.
