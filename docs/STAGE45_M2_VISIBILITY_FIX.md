# Stage45 M2 — Fang failure visibility fix

**M2: PASS.** Every Fang kind now logs a failed session setup, bounded and
classed, and `fang active` counts the failures. Plain TCP no longer logs a
line per connection. Discovery and sync report a local QUIC dial failure as
`unreachable`. A QUIC Fang resolves a hostname peer once, when it opens.
No wire, trust, Target Policy or receiver change.

## 1. Change

| Option | Where | Change |
|---|---|---|
| L1 (S45-D1) | `transport/fang_health.rs` (new); QUIC (`quic_fang.rs`), encrypted TCP (`tcp_encrypted.rs`), routed (`relay.rs`), failover (`failover.rs`) Fangs | one `FangHealth` per Fang, carried on its `FangCancellation`. A failed session **setup** is logged with the Stage42 `RateLimitedLog` (the first, then at most one line per 10 s per Fang and class, with the count suppressed in between): `🦷 <fang id> to <peer> session setup failed (<class>): <error>`. Classes: `dial` (refused, no route, refused locally, no answer), `handshake` (reached, not established), `target` (plain TCP). Routed sessions use the existing `established` signal, so a failure after the destination's ACK is not counted. Failover records one failure per local connection when no provider establishes a session: `handshake` if any provider was reached, otherwise `dial`, with every provider's error |
| D2 (S45-D2) | `tcp_plain.rs` | the target connect error and timeout go through the limiter as class `target`; the pipe error after a session is established is also limited but not counted |
| S1a (S45-D3) | `quic_fang::dial`, shared by `connect_v3_on` (Fangs) and `rpc_quic` (discovery, sync) | a QUIC dial refused locally is `AddrNotAvailable` (`hs::local_dial_failed`); no answer within the read window, or a quinn `TimedOut`, is `TimedOut` (`hs::no_answer`). Everything after the dial stays `hs::rejected()`. Discovery's existing `failure_status` then reports `unreachable`, as `docs/V1_CLI_CONTRACT.md` defines it |
| C1 (S45-D4) | `control.rs` `fang.list`, `fang_registry::health_of` | each entry gains `sessions_failed` and `last_failure` (`null` or `{"class", "seconds_ago"}`). Additive; `docs/V1_CLI_CONTRACT.md` documents both and the log rate |
| H1 (S45-D5) | `main.rs` `resolve_quic_peer`, `quic_server_for`; `quic_fang::resolve_server`; `open_quic_fang` now takes a `SocketAddr` | a QUIC peer's hostname is resolved once, before the daemon's state lock is taken (bounded by the 5 s read window), first address (S44-D4). If it does not resolve, or the Pack address changed meanwhile, the open fails with `QUIC_FANG_FAILED` and the reason. The per-connection parse and its `bad server addr` line are gone |

One consequence of S1a beyond the Fang: a routed discovery or sync query
whose relay does not answer over a QUIC hop now reports `unreachable`
(no connection was made) where it reported `rejected`; a TCP hop already
did. Every refusal by a reached node still reports `rejected`.

## 2. Unit tests (8 new, 1 enabled)

| Test | Shows |
|---|---|
| `fang_health::tests::setup_failures_are_classed_dial_or_handshake` | the dial kinds, `no_answer` and `local_dial_failed` are `dial`; `rejected`, reset, EOF and TLS errors are `handshake` |
| `fang_health::tests::failures_are_counted_and_logged_at_a_bounded_rate_per_class` | 1000 failures in one class: one line, 999 suppressed; each class has its own limiter; count and `last_failure` |
| `fang_health::tests::established_session_errors_are_logged_but_not_counted` | plain-TCP pipe errors are limited and not counted |
| `quic_fang::tests::a_refused_quic_session_is_a_handshake_failure` | no grant: reset, one `handshake` failure; a granted session counts nothing |
| `quic_fang::tests::an_unanswered_quic_session_is_a_dial_failure` | dead peer: one `dial` failure |
| `quic_fang::tests::quic_server_addresses_resolve_once` | literals pass through; `localhost` resolves; a `.invalid` name fails |
| `control::…::fang_list_reports_failed_session_setups` | `fang.list` shows 0 / `null`, then 3 and `target`; `state` stays `Active` |
| `control::…::quic_fang_resolves_a_hostname_peer_at_open` | an unresolvable QUIC peer fails the open with `QUIC_FANG_FAILED`; `localhost` opens |
| `discovery::tests::a_local_quic_dial_failure_is_unreachable` (M1, now enabled) | `unreachable` |

## 3. Lab on the M2 build

Lab build `werewolfd`
`405131f44c0f931c215eec4413def39cdf520c9bbc6511de9afeff7c8f55060a`.
`tools/stage45/visibility.py`, the M1 scenarios, 10 clients each (flood
500). Results are in `target/stage45-m2/m2-visibility.json` and
`m2-hostname4.json`. `sessions_failed` is 11 where 10 clients failed: the
lab's readiness probe of the Fang listener is also a failed session.

| Scenario | M1 (`main`) | M2 |
|---|---|---|
| `grant:quic`, `grant:tcp`, `grant:routed`, `grant:failover` | reset; **0 lines**; `fang list` unchanged | reset; **1 line**, `handshake`; `sessions_failed` 11 |
| `dial:quic`, `dial:routed` (5 s each) | reset; 0 lines | reset; 5 lines in 50 s, `dial` ("no answer from peer"), with the suppressed count |
| `dial:tcp` | reset; 0 lines | reset; 1 line, `dial` ("Connection refused") |
| `dial:failover` (10 s each) | reset; 0 lines | reset; 10 lines in 100 s, one per interval, each naming both providers' errors |
| `dial:tcp-plain` | reset; 11 lines | reset; 1 line, `target` |
| `flood:tcp-plain`, 500 | **501 lines, 52,605 bytes** | **1 line, 119 bytes** |
| `hostname:quic` (`localhost`) | `pack add` ok; open ok; reset; `bad server addr` per connection, no Fang id | open ok (resolved at open); reset; 5 lines, `dial`, with Fang id; counted |
| `hostname:tcp` (`localhost`) | echo | echo |
| `hostname:quic`, `hostname:tcp` (`wolf-b.stage45` → `127.0.0.1` only, `--sandbox-hosts`) | — | **echo 10/10**, 0 lines, `sessions_failed` 0 |

With `localhost` the QUIC Fang still cannot reach B: on this host
`localhost` resolves to `::1` first, B listens on `127.0.0.1` only, and H1
takes the first address as discovery and sync do (S44-D4). The owner kept
H1 as approved; this stays the recorded first-address limitation. The
failure is now visible, bounded, labelled and counted. A name that
resolves to the listener's address works over QUIC as over TCP. The
`--sandbox-hosts` run uses a private `/etc/hosts` in an unprivileged
namespace (as the Stage44 sandbox); the host was not touched.

The daemons stayed up in every scenario, the log guard never triggered,
and host `/tmp` stayed at 5%.

## 4. Existing checks

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --workspace --all-targets --all-features -D warnings`:
  PASS.
- `cargo test --workspace --all-features`: **437 passed**, 0 failed, 0
  ignored (428, plus 8 new tests and the M1 test now enabled).
- `scripts/ci-local.sh` full (`target/ci-local/20261110T205506Z`): **13
  of 13 PASS**, including the discovery, sync, routed-Fang and provider
  gates.

## 5. Next

M3 (survey §6): the M1 scenarios ×3; the Stage42 flood scenarios, one
round; Stage32 S2 P0, S3 P0, S4 P0, S7 P0 (the QUIC dial path changed);
the gates through `ci-local`. Every historical verdict must hold.
