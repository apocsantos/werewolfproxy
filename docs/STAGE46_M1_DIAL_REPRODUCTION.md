# Stage46 M1 — dialling a hostname with several addresses, reproduced

**M1: D1–D5 CONFIRMED.**

- **D1:** a direct QUIC Fang, discovery and sync over QUIC use only the
  first address. All four shapes fail, including the two where the second
  address is the live peer. QUIC also cannot tell a refused first address
  from a silent one: both cost the full 5 s.
- **D2:** a routed QUIC hop to a relay given by name opens, then resets
  every session at once, labelled `handshake` ("handshake rejected").
- **D3:** a failover Fang whose QUIC provider is given by name is refused
  at open ("QUIC address invalid").
- **D4:** TCP falls through a refused first address on every path, but a
  silent first address uses the whole window and the second is never
  tried. On an encrypted-TCP Fang this costs **10 s** per client, not 5:
  the Stage32 F1 retry repeats the timed-out connect.
- **D5:** an encrypted-TCP Fang out of descriptors labels its setup
  failures `handshake: Too many open files`; the two new unit tests fail
  as expected (`handshake`, `rejected`).

No production source changed.

## 1. Baseline

- `main` `dcbab52` (PR #139 merged). Lab debug `werewolfd`
  `405131f44c0f931c215eec4413def39cdf520c9bbc6511de9afeff7c8f55060a`,
  the Stage45 M2 binary (no source change since). Every result records
  that hash.
- Owner decisions S46-D0–D5 as recommended: A1 resolve at open and keep
  every address; R1 staggered race, 250 ms, inside the existing window;
  E1 one QUIC endpoint per family, created on first use; every QUIC path
  and TCP through one dialer; L1 local resource errors are `dial` and
  `unreachable`.

## 2. Method

`tools/stage46/dial.py`, run 2026-11-10 23:48–23:56 UTC, once per
scenario, each in a fresh lab with the 50 MB log guard. The lab runs in an
unprivileged namespace sandbox (private root, `/tmp`, `/etc/hosts` and
`/etc/gai.conf`; a `dummy` interface that drops `10.46.0.0/24`). Every
daemon listens on `127.0.0.1` only. The tool checks the resolver order
before it starts, and each result records the order it saw:

| Shape | Addresses, in order | First address |
|---|---|---|
| `fast` | `127.0.0.2`, `127.0.0.1` | nothing listens: TCP refused at once, UDP gets port unreachable |
| `silent` | `10.46.0.9`, `127.0.0.1` | dropped, no answer |
| `family` | `::1`, `127.0.0.1` | nothing listens; the host's own `localhost` order |
| `dead` | `127.0.0.2`, `10.46.0.9` | no address answers |

Paths, with the name in place of a Pack address (or, for plain TCP, the
service target): a direct QUIC and encrypted-TCP Fang to B; a plain-TCP
Fang; a failover Fang with providers B (by name, QUIC) then C (literal);
a Fang to C routed via relay B (by name), QUIC and TCP hop; A's
`discover refresh` of B; B's `governance sync` from A, over QUIC and TCP.
Each Fang scenario runs 5 clients one at a time and records each
outcome and its time.

D5: `tools/stage42/fang_flood.py fd:tcp fd:quic --nofile 64
--keep-temp`, then B's `session setup failed` lines (2026-11-10 23:56–23:58
UTC).

Results in `target/stage46-m1/`: `m1-dial.json`, `m1-dial.log`,
`d5-fd64.json` and `d5-fd64-setup-lines.txt`.

## 3. Results

`sessions_failed` is 6 where 5 clients failed: the lab's readiness probe
is a failed session too (Stage45).

### Fangs (per client: outcome, seconds; 5 clients each)

| Path | `fast` | `silent` | `family` | `dead` |
|---|---|---|---|---|
| QUIC | reset, 5.0; `dial` | reset, 5.0; `dial` | reset, 5.0; `dial` | reset, 5.0; `dial` |
| encrypted TCP | **echo**, 0.01 | reset, **10.0**; `dial` ("tcp connect timeout") | **echo**, 0.01 | reset, 10.0; `dial` |
| plain TCP (target by name) | **echo**, 0.00 | reset, 5.0; `target` | **echo**, 0.00 | reset, 5.0; `target` |
| failover (QUIC provider by name) | open refused | open refused | open refused | open refused |
| routed, QUIC hop (relay by name) | reset, **0.0**; `handshake` | same | same | same |
| routed, TCP hop (relay by name) | **echo**, 0.03–0.18 | reset, 5.0; `dial` | **echo**, 0.08–0.14 | reset, 5.0; `dial` |

- QUIC: one line per Fang per 10 s, `dial` "no answer from peer";
  `sessions_failed` 6. With `fast` the live address is second, and with
  `family` this is the `localhost` case.
- Failover: `service connect` exits 70, "wolf-b: QUIC address invalid";
  nothing is listened on.
- Routed QUIC hop: the Fang opens; each session fails before any packet,
  because the relay's address is parsed per session and a name does not
  parse. It is logged once, `handshake` "handshake rejected", and
  counted; the cause is not named.
- Plain TCP accepts a service target by name and resolves it per session,
  as encrypted TCP does.

### Discovery and sync (status, seconds)

| Path | `fast` | `silent` | `family` | `dead` |
|---|---|---|---|---|
| `discover refresh`, QUIC | `unreachable`, 5.0 | `unreachable`, 5.0 | `unreachable`, 5.0 | `unreachable`, 5.0 |
| `discover refresh`, TCP | **`ok`**, 0.02 | `unreachable`, 5.0 | **`ok`**, 0.02 | `unreachable`, 5.0 |
| `governance sync`, QUIC | `unreachable`, 5.0; nothing applied | same | same | same |
| `governance sync`, TCP | **`ok`**, applied 1 | `unreachable`, 5.0 | **`ok`**, applied 1 | `unreachable`, 5.0 |

The Stage45 S1a status holds: each failure is `unreachable`, none
`rejected`.

### D5, descriptors (`nofile` 64)

- `fd:tcp`: B logged 4 lines in the run, each
  `session setup failed (handshake): Too many open files (os error 24)`,
  with 111–121 suppressed between them.
- `fd:quic`: B's failures were `handshake: stream reset by peer`, the
  receiver's quota reset (Stage42), correctly labelled. A QUIC Fang's
  endpoint exists from the open, so its sessions do not open sockets and
  do not hit `EMFILE` on the dial. Discovery and sync create an endpoint
  per query, and `client_endpoint` maps its errors to `hs::rejected()`
  (survey §3), so there a descriptor shortage reports `rejected`.
- Unit tests added, `#[ignore]` until M2:
  - `transport::fang_health::tests::local_resource_errors_on_the_dial_are_dial_failures`:
    `EMFILE`, `ENFILE`, `ENOBUFS`, `ENOMEM` are `dial`; today `Handshake`
    (errno 24 first);
  - `discovery::tests::a_local_resource_error_is_unreachable`: the same
    errors are `unreachable`; today `rejected`.

The daemons stayed up in every scenario, the log guard never triggered,
and host `/tmp` stayed at 5%. The two kept Stage42 labs were removed after
their lines were saved.

## 4. Consequences for M2

- **QUIC needs the stagger even when the first address refuses.** quinn
  does not treat a UDP port-unreachable as a failure, so a refused first
  address costs the window like a silent one. A sequential dialler (R2)
  would not have fixed `localhost`; R1 does.
- **The F1 retry and the new TCP dialler.** With R1 inside `open_tcp`, a
  silent first address costs 250 ms and the second answers, so F1 is not
  reached. When every address is silent, F1 still retries the whole race
  once (10 s, as today). M2 keeps F1 unchanged; its scope is the Stage32
  certification.
- **Routed QUIC hop:** the relay's address is resolved at open (A1), so
  the per-session parse and its unnamed `handshake` failure go.
- **Failover:** a QUIC provider's name is resolved at open, as a direct
  QUIC Fang's is; the open fails only if it does not resolve.
- **D5 covers two places:** the classifier (`of_setup`, `failure_status`,
  by raw OS error) and `client_endpoint`, which must keep the error kind.

## 5. Next

M2 as approved (survey §6), with the unit tests above enabled.
