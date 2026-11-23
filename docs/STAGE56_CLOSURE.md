# Stage56 — local Fang identity closure

**Status: Stage56 CLOSED; M1–M4 merged, 2026-11-23.** The owner merged
[M4 PR #182](https://github.com/apocsantos/werewolfproxy/pull/182) as
`ec30713671bea757cf81c4729d5839bf8cd4a751`. This closes the approved Linux
Unix endpoint work for per-UID local Fang admission. The change is additive;
the frozen release artifacts and legacy TCP behavior remain unchanged.

## Delivery

| Milestone | Owner-merged delivery | Record |
|---|---|---|
| M1 identity survey | [Survey](STAGE56_LOCAL_FANG_IDENTITY_SURVEY.md) | Existing TCP loopback cannot distinguish local users |
| M2 endpoint contract | [PR #180](https://github.com/apocsantos/werewolfproxy/pull/180) | [Design](STAGE56_M2_LOCAL_UNIX_DESIGN.md) |
| M3 Linux Unix endpoint | [PR #181](https://github.com/apocsantos/werewolfproxy/pull/181), `5eeafe2` | [Implementation](STAGE56_M3_UNIX_ENDPOINT_IMPLEMENTATION.md) |
| M4 transport, UID and CLI regressions | [PR #182](https://github.com/apocsantos/werewolfproxy/pull/182), `ec30713` | [Regression record](STAGE56_M4_UNIX_TRANSPORT_REGRESSION.md) |
| Closure | This record | Documentation only |

The Linux listener uses kernel `SO_PEERCRED` UID identity in the shared
16-session per-source admission policy. The public `service connect` command
accepts `--local-endpoint unix:/absolute/path`. Direct QUIC, encrypted TCP,
plain TCP, routed QUIC-hop and provider-failover paths pass exact bytes over
Unix streams and clean up their socket paths. The native namespace test
observed a UID 1 client from a UID 0 acceptor.

## Reviewed validation

- The M4 report records all five transport paths, exact payloads, socket
  cleanup, daemon survival, the UID 0/UID 1 credential check and successful
  legacy TCP service-gate coverage.
- Workspace tests passed with 95 core, 10 CLI and 402 daemon tests; the
  10,000-session QUIC stress case took 247.66 seconds. The later UID harness
  passed separately, as did all 44 Fang-acceptor tests.
- Hosted Rust CI passed formatting, Clippy, release build and tests on the
  final PR head. Clippy initially found a needless `return` in the shared
  endpoint validator; the branch fix is included in PR #182.
- The local fast CI gate, `cargo fmt --check` and `git diff --check` passed.

The disposable native Unix-socket labs require the host filesystem ownership
view. The managed sandbox maps `/` to UID/GID 65534, so the daemon's private
directory ancestry check rejects it before readiness. The successful host-view
runs are recorded in the M4 report.

## Remaining limits

- The Unix endpoint is Linux-only. Windows and other Unix credential APIs are
  unsupported; container UID mapping and non-Linux behavior are not certified.
- TCP loopback still has no local-user identity and keeps its existing shared
  admission behavior. This work does not change TCP fairness.
- The five-path lab verifies byte-stream correctness and cleanup, not
  throughput, descriptor ceilings or WAN behavior. Namespace coverage proves
  kernel UID attribution for one two-UID case, not every host/container policy.
- Payload idleness does not establish application-level idleness. The existing
  60-second idle rule and active-session protection retain their documented
  limits.

See [known limitations](../KNOWN_LIMITATIONS.md) and [current status](STATUS.md).
