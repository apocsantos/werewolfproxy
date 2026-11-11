# Stage46 M2 — dialling every resolved address

**M2: PASS.** Every dial of a Pack address now tries every address the
name has, as a staggered race inside the window the single dial had. The
four behaviours M1 found are one: QUIC Fangs, routed QUIC hops, failover
QUIC providers, discovery and sync reach a peer whose first address is
refused, silent or of the other family, within about 0.3 s. Local
resource errors are `dial` and `unreachable`. No wire, trust, Target
Policy or receiver change.

## 1. Change

| Decision | Where | Change |
|---|---|---|
| R1 (S46-D2) | `transport/dial.rs` (new): `race`, `interleave`, `resolve_all`, `connect_tcp`, `Resolved` | the first address at once; the next 250 ms later, or at once when the previous fails; families alternate from the first address's (RFC 8305 §4–5, without its DNS part); the first to connect wins and the rest are aborted. When all fail, the error of a peer that was reached wins over dial errors, so a refusal still reads `handshake`. The caller's window bounds the race: `hs::READ_WINDOW` (5 s) for QUIC, the existing 5 s for TCP. A literal address is one attempt, as before |
| E1 (S46-D3) | `quic_fang::QuicPeer` | a peer's addresses and up to two client endpoints, one per family. The first address's family is bound when the peer opens, so a bind failure still fails the open; the other only when one of its addresses is first tried. `connect_v3_on` takes a `QuicPeer`; the dial's per-address error kinds (Stage45 S1a) are unchanged |
| A1 (S46-D1) | `main.rs` `quic_address_for`, `resolve_fang_names`; `relay::resolve`, `failover::resolve` take a `Resolved` | before the state lock, every QUIC hostname the open will dial is resolved, concurrently, each within 5 s: the peer's own address, a QUIC relay's, every QUIC provider's (and its relay's). Every address is kept. A name that does not resolve fails the open: `QUIC_FANG_FAILED`, `FANG_ROUTE_INVALID` or `FANG_BAD_QUIC_ADDRESS`, each with the reason. Stage45's `resolve_server` (first address) is gone |
| P-all (S46-D4) | QUIC Fang, routed QUIC hop (`relay::relay_peer`), failover QUIC provider, discovery and sync (`rpc_quic` takes the address list; `operator.rs` resolves with `resolve_all`) | one dialer. Discovery's routes are built without resolving, so a QUIC relay there is resolved when queried |
| T1 (S46-D4) | `tcp_encrypted::open_tcp` (encrypted-TCP Fangs, TCP hops, failover TCP providers, discovery and sync over TCP), `tcp_plain` (target) | `dial::connect_tcp` instead of tokio's sequential connect, inside the same 5 s. The Stage32 F1 retry is unchanged: it repeats the whole race once when the TCP connect fails |
| L1 (S46-D5) | `handshake::no_connection_made`, used by `FailureClass::of_setup` and discovery's `failure_status`; `client_endpoint` | `EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM` (by raw OS error) join the dial kinds; a QUIC endpoint that cannot bind is `hs::local_dial_failed` (was `hs::rejected()`) |

`docs/V1_CLI_CONTRACT.md` says how a hostname is dialled, when it is
resolved, and the new `dial` / `unreachable` cases.

## 2. Unit tests (16 new, 2 enabled, 1 removed)

| Test | Shows |
|---|---|
| `dial::tests::addresses_interleave_by_family_from_the_first` | order, alternation, duplicates |
| `dial::tests::a_single_address_is_one_attempt` | a literal is one attempt, no delay |
| `dial::tests::a_silent_first_address_costs_the_attempt_delay` | second starts at 250 ms, wins |
| `dial::tests::a_refused_first_address_starts_the_next_at_once` | a failure starts the next at once |
| `dial::tests::the_first_success_wins_and_later_addresses_never_start` | no extra attempts |
| `dial::tests::attempts_are_staggered_while_none_answers` | 0, 250, 500, 750 ms |
| `dial::tests::when_every_address_fails_a_reached_peers_refusal_is_reported` | refusal over dial errors |
| `dial::tests::the_callers_window_bounds_a_race_of_silent_addresses` | unbounded only by the caller |
| `dial::tests::names_resolve_to_every_address_and_literals_pass_through` | `resolve_all`, `Resolved` and its three answers |
| `dial::tests::tcp_tries_the_next_address_when_the_first_is_refused` | real sockets |
| `quic_fang::tests::a_quic_fang_falls_back_to_the_next_address` | dead IPv4 or `::1` first: established in under 2 s, nothing counted |
| `quic_fang::tests::when_no_address_answers_the_session_is_one_dial_failure` | three dead addresses: one `dial` failure, one window |
| `relay::tests::a_named_quic_relay_is_resolved_at_open` | QUIC hop takes the resolved list; unresolved fails the open; TCP hop keeps the name |
| `failover::tests::a_named_quic_provider_is_resolved_at_open` | list kept; does-not-resolve and retry reasons |
| `discovery::tests::a_quic_query_falls_back_to_the_next_address` | dead address first, then the node |
| M1, now enabled: `fang_health::tests::local_resource_errors_on_the_dial_are_dial_failures`, `discovery::tests::a_local_resource_error_is_unreachable` | L1 |
| removed: `quic_fang::tests::quic_server_addresses_resolve_once` | Stage45 `resolve_server`, replaced by `resolve_all` |

## 3. Lab on the M2 build

Lab build `werewolfd`
`844670206ae6228a6bcfc18771daa43ecc68847328729d8c4c853168674c6b9e`.
`tools/stage46/dial.py`, the M1 matrix, 5 clients each (2026-11-11
UTC). Results in `target/stage46-m2/`.

### Fangs (per client: outcome, seconds)

| Path | `fast` | `silent` | `family` | `dead` |
|---|---|---|---|---|
| QUIC | **echo**, 0.30 (M1: reset, 5.0) | **echo**, 0.31 (M1: reset, 5.0) | **echo**, 0.30 (M1: reset, 5.0) | reset, 5.0; one `dial` line |
| encrypted TCP | echo, 0.01 | **echo**, 0.28–0.31 (M1: reset, 10.0) | echo, 0.01 | reset, 10.0; `dial` (F1 retry, unchanged) |
| plain TCP | echo, 0.00 | **echo**, 0.25 (M1: reset, 5.0) | echo, 0.00 | reset, 5.0; `target` |
| failover, QUIC provider by name | **echo**, 0.27–0.31 (M1: open refused) | **echo**, 0.31 | **echo**, 0.30 | echo via C, 5.05–5.07 (B's window, then C) |
| routed, QUIC hop (relay by name) | **echo**, 0.33–0.36 (M1: reset, 0.0, `handshake`) | **echo**, 0.34–0.35 | **echo**, 0.33–0.34 | reset, 5.0; **`dial`** "no answer from peer" (M1: `handshake`) |
| routed, TCP hop | echo, 0.03–0.09 | **echo**, 0.35–0.39 (M1: reset, 5.0) | echo, 0.08–0.13 | reset, 5.0; `dial` |

`sessions_failed` is 0 wherever every client echoed, 6 where 5 failed (the
lab's readiness probe counts, Stage45). A failover Fang counts nothing when
another provider serves (Stage45).

### Discovery and sync (status, seconds)

| Path | `fast` | `silent` | `family` | `dead` |
|---|---|---|---|---|
| `discover refresh`, QUIC | **`ok`**, 0.32 | **`ok`**, 0.33 | **`ok`**, 0.31 | `unreachable`, 5.0 |
| `discover refresh`, TCP | `ok`, 0.02 | **`ok`**, 0.30 | `ok`, 0.02 | `unreachable`, 5.0 |
| `governance sync`, QUIC | **`ok`**, applied 1 | **`ok`** | **`ok`** | `unreachable`, 5.0 |
| `governance sync`, TCP | `ok` | **`ok`**, 0.33 | `ok` | `unreachable`, 5.0 |

### `localhost` (Stage45 tool, host resolver: `::1` first)

`tools/stage45/visibility.py hostname:quic hostname:tcp` on the host:
**QUIC echo 10/10** (Stage45 M2 and M3: reset, 5 `dial` lines), TCP echo
10/10, no lines.

### D5 (`fd:tcp`, `fd:quic`, `nofile` 64)

- `fd:tcp`: B's descriptor-exhaustion lines now read
  `session setup failed (dial): Too many open files (os error 24)`, with
  111–134 suppressed between them (M1: `handshake`). Because they no
  longer share the `handshake` limiter, the receiver's refusals during the
  same flood now get their own bounded `handshake` line ("peer closed
  connection without sending TLS close_notify"); in M1 they were inside
  the suppressed count.
- `fd:quic`: unchanged, `handshake: stream reset by peer` (the receiver's
  quota, Stage42).
- Lines in `d5-fd64-setup-lines.txt`; the kept labs were removed after.

The daemons stayed up in every scenario, the log guard never triggered,
and host `/tmp` stayed at 5%.

## 4. Existing checks

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --workspace --all-targets --all-features -D warnings`:
  PASS.
- `cargo test --workspace --all-features`: **453 passed**, 0 failed, 0
  ignored (437, plus 16 new and the 2 M1 tests enabled, less the
  replaced Stage45 test).
- `scripts/ci-local.sh` full, rebased on `main` with PR #140
  (`target/ci-local/20261111T123242Z`): **13 of 13 PASS**. The run before
  the rebase (`20261111T121744Z`) passed every check but the audit, which
  failed on RUSTSEC-2026-0486 alone (fixed by #140). M2 changes no
  dependency.

## 5. Next

M3 (survey §6): M1 ×3; Stage45 visibility ×1 (`localhost` now expected to
echo over QUIC); Stage44 IPv6 ×1; Stage42 floods ×1; Stage32 S2, S3, S4,
S7 P0 and S2 P1 (T1 changes the TCP dial); `ci-local` full.
