# Stage49 M2 — a refused QUIC dial fails at once

**M2: PASS.** With each QUIC dial attempt, a separate connected UDP socket
sends one 1-byte datagram; the peer's ICMP port-unreachable on it ends
the attempt at once as a refused dial. Every QUIC path to a peer with
nothing listening now fails in 0.00–0.01 s instead of 5 s, failover
reaches a live provider behind one or two refused ones in 0.02–0.03 s
instead of 5.05 / 10.05 s, and a name whose first address refuses
connects in 0.02 s instead of 0.30 s. Silent peers and every TCP path are
unchanged. No Werewolf message, handshake, trust, Target Policy, receiver
or dependency change; the probe datagram is the one new thing on the
wire (S49-D1, R1).

## 1. Change (R1)

| Where | Change |
|---|---|
| `quic_fang.rs` `QuicPeer::dial` | each address's attempt races the QUIC dial against `refusal(address)`; the dial's result wins unless the refusal comes first |
| `quic_fang.rs` `refusal`, `refusal_probe` | bind a UDP socket of the address's family (`client_bind`), connect it to the address, send one byte (`0x00`), wait for `ECONNREFUSED`. A datagram back is ignored; any other error, or a socket that cannot be opened (descriptor exhaustion), leaves the future pending, so the QUIC dial runs exactly as before |
| `handshake.rs` `refused_by_peer` | the error: `ConnectionRefused`, "refused: nothing listening at the peer's address". `no_connection_made` already covers it: `dial` in a Fang's log, `unreachable` in discovery and sync |

Every QUIC dial goes through `QuicPeer::dial` (direct and routed Fangs,
failover providers, discovery, sync), so every path gets it. A refused
attempt inside a hostname's race (Stage46) now fails, so the race starts
the next address at once instead of on the 250 ms stagger.

## 2. Unit tests (3 new)

| Test | Shows |
|---|---|
| `quic_fang::tests::a_quic_dial_to_a_closed_port_is_refused_at_once` | a QUIC dial to a closed UDP port fails in under 1 s with `ConnectionRefused`, counted as no connection made |
| `quic_fang::tests::the_refusal_probe_waits_on_an_open_port` | the probe does not complete in 300 ms on a port with a socket bound (live or silent), completes at once on a closed one, and sends exactly one 1-byte datagram |
| `quic_fang::tests::a_refused_first_address_does_not_wait_for_the_stagger` | two refused addresses fail together in under 200 ms (less than the 250 ms stagger) |

## 3. Sandbox (`tools/stage49/refused.py`, 3 clients each)

2026-11-12 07:12–07:17 UTC, this build's lab `werewolfd` `c484a543…` (M1
plus the uncommitted change, as the lab's provenance records). Results in
`target/stage49-m2/`.

| Path | `refused` | M1 | `blackhole` | `fast` | M1 `fast` |
|---|---|---|---|---|---|
| QUIC, direct | **reset, 0.00 s**; `dial`, "refused: nothing listening at the peer's address" | 5.0 s | reset, 5.0 s | **echo, 0.02 s** | 0.30–0.31 s |
| failover, B (QUIC) then C | **echo via C, 0.02 s** | 5.05–5.06 s | 5.05 s | echo, 0.02–0.03 s | 0.30–0.31 s |
| failover, B and C (QUIC) then D | **echo via D, 0.02–0.03 s** | 10.05 s | 10.05–10.06 s | echo, 0.02 s | 0.31 s |
| routed, QUIC hop | **reset, 0.00 s**; `dial` | 5.0 s | reset, 5.0 s | echo, 0.04 s | 0.33–0.36 s |
| discovery, QUIC | **`unreachable`, 0.01 s** | 5.01 s | 5.01 s | `ok`, 0.03 s | 0.32 s |
| sync, QUIC | **`unreachable`, 0.01 s** | 5.01 s | 5.01 s | `ok`, 0.04 s | 0.33 s |
| TCP paths (direct, routed, discovery, sync) | 0.00–0.01 s | same | unchanged (direct 10.0 s, others 5.0–5.01 s) | 0.01–0.08 s | same |

Every M1 criterion holds: QUIC `refused` under 0.1 s on every path,
failover under 0.1 s to C and D, `fast` at a literal's 0.02–0.04 s,
`blackhole` and TCP unchanged. Every daemon stayed up.

**Datagrams, again on this build:** 10,000 1-byte datagrams to a live
QUIC receiver in 0.10 s: no log line, 0.19 s of its CPU, every session
before and after echoed (0.02 / 0.04 s).

## 4. Existing checks

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --all-features -D warnings`: PASS.
- `cargo test --workspace --all-features`: **459 passed**, 0 failed (456
  and the three new tests). The run spans 23:21–07:12 UTC because the
  machine slept (no journal entries between 23:25 and 07:05); the
  `werewolfd` suite took 324 s of test time (Stage48 M2: 420 s), its known
  slow test (`short_lived_local_fang_connections_finish_quic_and_release_authority`)
  over 60 s as before.
- `scripts/ci-local.sh` full (`target/ci-local/20261112T071727Z`): **13
  of 13 PASS**.

## 5. What this does not change

- **Where ICMP does not arrive** (filtered on many WAN paths), a refused
  QUIC peer still costs the 5 s window: the probe sees nothing and the
  dial runs as before.
- **A silent peer** (no answer at all) still costs 5 s; that is the
  Stage45 item, unchanged.
- **Forged ICMP:** an attacker who can forge an ICMP port-unreachable for
  the probe's source port can make one attempt fail early. The outcome is
  that of a dead peer (failover moves on, the client is reset).
- **One more UDP socket per QUIC attempt**, closed when the attempt ends.

## 6. Next

M3 (survey §7): the M1 tool ×3; Stage46 matrix ×1; Stage48 tool ×1;
Stage42 `fd:quic` floods (the probe socket under exhaustion); Stage32 S2,
S3, S4, S7 P0; `ci-local` full.
