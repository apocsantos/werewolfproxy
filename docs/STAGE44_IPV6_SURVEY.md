# Stage44 — IPv6 (survey)

**Status: survey, for owner review.** Docs only. No production source,
harness, dependency or verdict changes.

IPv6 has never been run: `KNOWN_LIMITATIONS.md` lists it under "Not
run", and every lab, gate and chaos cell uses IPv4 loopback. This survey
reads the code paths that take, store or dial an address, says what
should and should not work over IPv6 today, and proposes how to test and
fix it without a WAN.

## 1. Baseline

- `main` `1222c1f` (PR #128 merged, Stage43 CLOSED).
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18.
- Tag `v1.0.0-rc.2` frozen. S3-P2 and S2-P4 stay EXCLUDED.
- Lab feasibility, checked on this host: IPv6 is enabled
  (`disable_ipv6` 0). In an unprivileged `unshare -rn` network namespace,
  `lo` carries `::1`, unique-local addresses (`fd43::1/128`,
  `fd43::2/128`) can be added, and a TCP connection between them works.
  The Stage32/Stage43 sandbox can therefore host IPv6 labs without root,
  without touching the host's interfaces and without a WAN.

## 2. What the code does with addresses (from reading; not yet run)

| Path | Where | IPv6 today |
|---|---|---|
| Peer address in state | `state_validation.rs` `endpoint()`: `tcp://` or `quic://`, then `rsplit_once(':')`, non-empty host, port ≠ 0 | accepts `quic://[fd43::1]:4433`; also accepts hostnames and unbracketed text, checked later |
| **QUIC client endpoint** | `quic_fang.rs` `client_endpoint()`: `Endpoint::client("0.0.0.0:0")`. Used by the QUIC Fang, failover's QUIC leg, discovery (`discovery.rs:476`) and governance sync over QUIC | **expected to fail.** Quinn 0.11 refuses an IPv6 destination on an IPv4 endpoint (`quinn/src/endpoint.rs`: `if addr.is_ipv6() && !endpoint.ipv6 { Err(InvalidRemoteAddress) }`). Every QUIC session, discovery query and sync to an IPv6 peer would fail at once, before any packet is sent |
| QUIC Fang server address | `quic_fang.rs:89`: `quic_server.parse::<SocketAddr>()` per connection | an IPv6 literal parses; a hostname fails (IPv4 too) |
| QUIC listener (receiver) | `transport/quic.rs:117`: `Endpoint::server(config, addr)` | `[::]:p` should be dual-stack on Linux (`bindv6only` 0); `[fd43::1]:p` IPv6 only. Not run |
| Encrypted-TCP and plain-TCP dial | `TcpStream::connect` on the peer string; TLS name from the IP (`tcp_encrypted.rs:394`); the custom verifier ignores the name (`tls_identity.rs:194`) | should work; not run |
| TCP listeners, Fang local bind | `TcpListener::bind(&str)` | `[::1]:p` and `[fd43::1]:p` should work; not run |
| Discovery and sync dial | `discovery/operator.rs`: `lookup_host(address)`, **first result only** | literal works for TCP; QUIC hits the client-endpoint failure; a hostname whose first answer is AAAA hits it too |
| Contact records | `werewolf-core/contact.rs` `valid_address`: `quic://`/`tcp://` + literal `SocketAddr`, no hostname, not unspecified | accepts `[fd43::1]:p`; carried as text, so no IPv4-only encoding. Not run |
| Published services | `published.rs` `check_target`: literal, canonical spelling | accepts `[::1]:22` (a test uses `[::]:22` as invalid) |
| Target Policy | `target_policy.rs`: grants for `[::1]:8080`; scoped and link-local IPv6 refused (`scope_id == 0`, not `fe80::/10`) | unit-tested for IPv6 parsing; never dialled in a lab |
| Routed Fang | `transport/relay.rs:88`: destination parsed as `SocketAddr` | literal accepted; relay hop inherits the QUIC failure when the hop is QUIC |
| Fang admission (Stage43) | `fang_accept.rs` `share_key`: `to_canonical()`; loopback exempt | IPv4-mapped handled and unit-tested. **Each IPv6 address is its own source** (see §3) |
| Non-loopback bind warning | `fang_accept.rs:330` | covers IPv6 (`::1` is loopback) |
| `werewolfctl service connect --bind` | `werewolfctl/src/main.rs:687`: `format!("{bind}:{local_port}")` | `--bind ::1` yields `::1:8080`, which is not a socket address; `--bind [::1]` works. A CLI defect |
| Test tooling | `tests/integration/*.py`, `tools/stage4*/` | 74 hard-coded `127.0.0.1`/`localhost`; no IPv6 lab exists |

No wire format, signed statement or state schema stores an address in a
binary IPv4 form: addresses travel and persist as text. So IPv6 support
needs no protocol or state-format change. That is a reading, to be
confirmed by M1.

## 3. Findings to test

- **H1 (expected defect, high impact):** QUIC to an IPv6 peer fails
  everywhere it is used (Fang, failover QUIC leg, routed QUIC hop,
  discovery, sync). QUIC is the default transport, so an IPv6-only peer
  is reachable only by `--transport tcp`/`tcp-plain`.
- **H2 (expected to work):** encrypted-TCP and plain-TCP Fangs, TCP
  discovery and sync, routed Fangs over a TCP hop, a Target Policy grant
  for an IPv6 target, and Fangs bound to `[::1]` or a ULA, all over
  IPv6.
- **H3 (Stage43 gap):** the per-source share counts each IPv6 address
  separately. One IPv6 host usually controls a whole /64 (privacy
  addresses, or any address it likes on its own prefix), so on a Fang
  bound to an IPv6 non-loopback address one host can take all 64 slots
  with four addresses. Stage43's scenario D is open again for IPv6.
- **H4 (CLI defect, low impact):** `service connect --bind ::1` builds an
  invalid local address.
- **H5 (dual-stack listener):** a receiver on `[::]:p` serves IPv4 and
  IPv6 peers; IPv4 peers then appear as IPv4-mapped addresses. Nothing
  found that authorises by peer address (trust is the Pack key), but
  logs and the Stage43 share see the mapped form (handled by
  `to_canonical`). To confirm.
- **Recorded, not proposed for change:** dial uses only the first
  resolved address (no Happy Eyeballs, no fallback between families).
  Contact records carry literal addresses only, so discovery never
  resolves names.

## 4. Options

### V1. QUIC client endpoint matching the destination's family (fixes H1)

Bind the client endpoint to `[::]:0` when the destination is IPv6 and
`0.0.0.0:0` when it is IPv4. The QUIC Fang creates its endpoint once per
Fang, before the first connection; it would parse `quic_server` once at
start (already a literal) and choose the family then. Discovery and sync
create an endpoint per query and already hold the resolved address.

- IPv4 behaviour is byte-for-byte unchanged: the same `0.0.0.0:0` socket.
- No protocol, trust, TLS or window change; only the local UDP socket's
  family.
- Alternative **V1b**: always bind a dual-stack `[::]:0`. One code path,
  but on a host with IPv6 disabled the bind fails and **all** QUIC stops,
  IPv4 included. Not recommended.

### V2. Per-prefix share for IPv6 (fixes H3)

In `share_key`, key a non-loopback IPv6 source by its /64 prefix
(IPv4 and IPv4-mapped unchanged). /64 is the smallest prefix normally
assigned to one host or link. A /56 or /48 delegated to one site is not
covered; a whole LAN behind one /64 shares 16 slots (the same trade
Stage43 made for IPv4 NAT).

### V3. Bracket IPv6 binds in `werewolfctl` (fixes H4)

Parse `--bind` as an `IpAddr` and build the local address with
`SocketAddr::new`, so `::1` and `[::1]` both work and a hostname is
refused at the CLI. A CLI-only change.

### V4. IPv6 lab tooling (needed for M1/M3 whatever is fixed)

A `tools/stage44/ipv6.py` that runs in the Stage43 namespace sandbox
(private root, private 512 MB `/tmp`, the 50 MB log guard), puts
`fd43::1`–`fd43::3` on `lo` and runs two or three Dens on IPv6 only.

### V5. Document only

Keep IPv4-only QUIC and say so. Not recommended: QUIC is the default
transport and the failure is total.

## 5. Recommendation

**V4 + V1 + V2 + V3.** V1 is the fix that matters; V2 closes the hole
IPv6 opens in Stage43's share; V3 is a one-line CLI fix. Record, do not
change: first-address-only dialling.

## 6. Proposed Stage44 contract (for owner review)

- **M1, reproduction** (sandbox only, no WAN, no live wolves; log guard
  kept), with V4, on `main`:
  - I1: QUIC Fang to a peer on `[fd43::2]` — expected to fail (H1);
  - I2: encrypted-TCP and plain-TCP Fangs to the same peer, echo and a
    1 MiB transfer by SHA-256 — expected to pass (H2);
  - I3: discovery advertise/refresh and governance sync between IPv6
    peers, QUIC and TCP;
  - I4: a routed Fang with an IPv6 relay and destination, QUIC and TCP
    hops;
  - I5: a receiver on `[::]:p` serving one IPv4 and one IPv6 peer (H5);
  - I6: a Fang bound to `[fd43::1]`, one host flooding from four
    addresses in one /64, a second host connecting (H3);
  - I7: `service connect --bind ::1` (H4).
  - M1 stops the stage if nothing reproduces.
- **M2, fix:** V1, V2, V3. Unit tests: an IPv6 destination gets an IPv6
  endpoint and an IPv4 one stays `0.0.0.0:0`; QUIC connect to `[::1]`
  in-process; /64 grouping (and IPv4/mapped unchanged); CLI bind
  spelling. Workspace tests, fmt, clippy, `scripts/ci-local.sh` full.
- **M3, regression:** the M1 scenarios ×3 over IPv6; the Stage43 idle
  and source scenarios and Stage42 flood scenarios (IPv4, unchanged
  path, one round each); Stage32 S2 P0/P1, S3 P0, S4 P0, S7 P0 (QUIC
  dial path touched); the routed-Fang gate. Every historical verdict must
  hold.
- **M4:** `KNOWN_LIMITATIONS` update (IPv6 moves from "Not run" to what
  was run in the sandbox) and closure.

## 7. Risks

- **QUIC dial path changes** (V1). Mitigated by keeping the IPv4 socket
  identical and re-running the QUIC chaos cells.
- **Sandbox is not a real IPv6 network.** No router advertisements, no
  path MTU changes, no real WAN. The record will say IPv6 was run in a
  namespace on loopback-class addresses only; WAN stays not authorised.
- **/64 grouping is a heuristic** (V2): larger delegations evade it, and
  shared /64s share a quota. Stated.
- **Tooling scope:** porting every gate to IPv6 is out of scope; V4 is a
  separate tool and the IPv4 gates stay as they are.

## 8. Owner decisions

| ID | Decision | Options | Recommended |
|---|---|---|---|
| S44-D0 | Approve Stage44 as scoped (§6) | approve, amend, reject | approve |
| S44-D1 | QUIC client endpoint (H1) | V1 family per destination; V1b dual-stack always; V5 document only | V1 |
| S44-D2 | IPv6 per-source share (H3) | V2 group by /64; keep per-address; other prefix | /64 |
| S44-D3 | CLI bind spelling (H4) | V3 fix; document only | V3 |
| S44-D4 | Dialling multiple resolved addresses | record only; add fallback between addresses | record only |
| S44-D5 | Where IPv6 is run | namespace sandbox only (no WAN); also WAN (not authorised today) | sandbox only |
