# Stage49 M1 — a QUIC peer with nothing listening, reproduced

**M1: P1 and P2 CONFIRMED; the probe datagram is harmless to a live
receiver.**

- **P1:** with nothing listening on the peer's port, every QUIC path waits
  the full window, exactly as for a peer that never answers: direct and
  routed Fangs reset each client at 5.0 s, failover reaches the next
  provider at 5.05–5.06 s (two refused: 10.05 s), discovery and sync
  report `unreachable` at 5.01 s. TCP fails in 0.00–0.01 s on the same
  paths.
- **P2:** a name whose first address refuses costs QUIC 0.30–0.36 s (the
  250 ms stagger before the next address), against 0.01–0.02 s on TCP and
  0.02 s for a QUIC literal.
- **Datagrams:** 10,000 1-byte UDP datagrams to a live QUIC receiver in
  0.07 s: no log line, 0.14 s of its CPU, every session before and after
  echoed, its Fang still active with no failure.

The only change is tooling: `tools/stage49/refused.py`.

## 1. Baseline

- `main` `2d1172a` (PR #155 merged). Lab debug `werewolfd` `b928e6ff…`,
  the Stage48 binary.
- Owner decisions S49-D0–D1 as recommended: approve; R1 (the refusal
  probe, one 1-byte datagram per QUIC attempt), on the reading that the
  datagram is accepted.

## 2. Tool

`tools/stage49/refused.py` (README beside it) runs in the Stage46 sandbox
with the Stage48 lab and names: `refused` (`127.0.0.2`, nothing
listens), `blackhole` (`10.46.0.9`, dropped), `fast` (`127.0.0.2`, then
`127.0.0.1`). Paths: direct, failover with one or two named providers,
routed, discovery, sync, over QUIC and TCP. `datagrams` opens a QUIC Fang
from A to B by literal address, runs 3 clients, sends 1-byte datagrams to
B's QUIC port from a connected socket, waits 1 s, and runs 3 more.

## 3. Results (2026-11-11 23:01–23:07 UTC, 3 clients each)

Results in `target/stage49-m1/` (`m1.json`, `m1.log`, `meta.txt`).

| Path | `refused` | `blackhole` | `fast` |
|---|---|---|---|
| QUIC, direct | **reset, 5.0 s**; `dial` | reset, 5.0 s; `dial` | echo, 0.30–0.31 s |
| TCP, direct | reset, 0.00 s; `dial` | reset, 10.0 s (F1, Stage48 P3) | echo, 0.01 s |
| failover, B (QUIC) then C | **echo via C, 5.05–5.06 s** | echo via C, 5.05–5.06 s | echo via B, 0.30–0.31 s |
| failover, B and C (QUIC) then D | **echo via D, 10.05 s** | echo via D, 10.05–10.06 s | echo via B, 0.31 s |
| routed, QUIC hop | **reset, 5.0 s**; `dial` | reset, 5.0 s; `dial` | echo, 0.33–0.36 s |
| routed, TCP hop | reset, 0.00 s; `dial` | reset, 5.0 s; `dial` | echo, 0.09–0.14 s |
| discovery, QUIC | **`unreachable`, 5.01 s** | `unreachable`, 5.01 s | `ok`, 0.32 s |
| discovery, TCP | `unreachable`, 0.01 s | `unreachable`, 5.01 s | `ok`, 0.02 s |
| sync, QUIC | **`unreachable`, 5.01 s** | `unreachable`, 5.01 s | `ok`, 0.33 s |
| sync, TCP | `unreachable`, 0.01 s | `unreachable`, 5.01 s | `ok`, 0.02 s |

Every daemon stayed up in every scenario.

### Datagrams to a live QUIC receiver

| Sent | Time | Refused by the kernel | B's log | B's CPU | Clients before / after | B's Fang |
|---|---|---|---|---|---|---|
| 10,000 × 1 byte | 0.07 s | 0 | **0 lines** | 0.14 s | echo 3/3 (0.02 s) / echo 3/3 (0.02–0.04 s) | active, 0 failed |

quinn discards a datagram too short to be a QUIC packet before any
connection sees it; at the daemon's normal log level nothing is written.
R1 sends one such datagram per QUIC attempt, so this is about 10,000
times the probe's load.

## 4. Consequences for M2

R1 as approved: with each QUIC dial attempt, a separate UDP socket
connected to the same address sends one 1-byte datagram; `ECONNREFUSED`
on it ends the attempt at once as a refused dial. Criteria:

- QUIC `refused` under 0.1 s on the direct, routed, discovery and sync
  paths; failover reaches C in under 0.1 s and D in under 0.2 s;
- QUIC `fast` close to a literal (well under the 0.30 s it takes now),
  since a refused first address ends its attempt and the race starts the
  next at once;
- `blackhole` and every TCP path unchanged.
