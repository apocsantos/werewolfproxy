# Stage46 — closure record

**Status: Stage46 CLOSED.** A peer address that is a hostname is now
dialled at every address it names. Every Fang kind, routed hop, failover
provider, discovery and sync uses one staggered race inside the window a
single dial had, so a refused, silent or other-family first address costs
about 0.3 s instead of the session. A local resource error on the dial is
`dial` and `unreachable`.

This record is documentation only: no production source, harness,
dependency, bound or historical verdict changes.

## 1. Baseline

- `main` after PR #143 (M3) merged.
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18. During Stage46,
  rustix moved from 1.1.4 to 1.1.6 for RUSTSEC-2026-0486 (PR #140, its
  own record), the only `Cargo.lock` change.
- Tag `v1.0.0-rc.2` is frozen and unchanged. The fix is **not** in the
  rc.2 archive.
- S3-P2 and S2-P4 stay EXCLUDED. Stage38 stays CLOSED.
- No wire, trust, Target Policy or receiver change.

## 2. Milestones

| Step | PR | Result | Record |
|---|---|---|---|
| Survey | #139 | D1 QUIC first address only, D2 routed QUIC hop to a named relay, D3 failover QUIC provider by name, D4 TCP blocked by a silent first address, D5 local resource errors labelled `handshake`. Recommended A1 + R1 + E1 + P-all + T1 + L1. S46-D0–D5 approved as recommended | [survey](STAGE46_DIAL_FALLBACK_SURVEY.md) |
| Dependency | #140 | rustix 1.1.6 (RUSTSEC-2026-0486 blocked every push); own PR by owner decision | [record](RUSTIX_1_1_6_REMEDIATION.md) |
| M1 test | #141 | D1–D5 CONFIRMED in a namespace sandbox with a private resolver: QUIC fails on every shape (refused is as slow as silent), encrypted TCP 10 s on a silent first address | [M1](STAGE46_M1_DIAL_REPRODUCTION.md) |
| M2 fix | #142 | M2-PASS (`4dee787`): `transport/dial.rs` race, `QuicPeer` (endpoint per family), open-time resolution for peers, relays and providers, every QUIC and TCP dial on the dialer, `hs::no_connection_made`. 453 tests; local CI 13/13 | [M2](STAGE46_M2_DIAL_FIX.md) |
| M3 regression | #143 | PASS: matrix ×3 identical; `localhost` over QUIC 10/10; Stage45, Stage44, Stage42 scenarios hold; Stage32 S2 P0/P1, S3 P0, S4 P0, S7 P0 3/3; local CI 13/13. Found the `fd:tcp` CPU observation (owner: record) | [M3](STAGE46_M3_REGRESSION.md) |
| M4 | this PR | `KNOWN_LIMITATIONS.md` updated; closure | — |

## 3. What remains open

- **CPU under descriptor exhaustion:** an encrypted-TCP Fang's daemon out
  of file descriptors uses 0.8–8.8% of a core, against 1.2–1.8% before
  Stage46 (M3 §5, A/B). Bounded, ends with the exhaustion, recovers. Not
  investigated.
- **F1 and a dead name:** an encrypted-TCP Fang where no address answers
  takes 10 s per client; the Stage32 F1 retry repeats the race.
- **Resolution timing:** QUIC resolves at open; a DNS change needs a
  reopen. TCP, discovery and sync resolve at each dial.
- **Where it was run:** namespace sandbox only, loopback and unique-local
  addresses; never a real dual-stack network or a WAN (not authorised).
- From Stage45: a dead QUIC peer costs each client the 5 s window; refusal
  reasons stay hidden from the initiator, by design. From Stage44: real
  IPv6 networks, the /64 share heuristic, failover Fangs over IPv6. From
  Stage43: the active loopback flood (F2), the shared peer quota (F5), the
  operator ceiling per Fang (F4) and half-close. From earlier stages: the
  initiator's stream windows (Stage41; the send window is the Stage34
  surface), the routed-QUIC regression root cause (Stage34), and the
  real-WAN relay-to-destination check (S39-D5).
- **Tooling:** `tools/stage46/dial.py` keeps the Stage42 50 MB log guard
  and always runs in its sandbox; the host's resolver files and
  interfaces are not touched.

**Not authorised:** tokio promotion, rc.3, release certification, WAN
testing.
