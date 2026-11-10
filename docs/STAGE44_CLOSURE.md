# Stage44 — closure record

**Status: Stage44 CLOSED.** IPv6 has now been run, and QUIC works over it.
The QUIC failure toward IPv6 peers and the IPv6 gap in the per-source
share were reproduced, fixed and regression-tested in a namespace sandbox.

This record is documentation only: no production source, harness,
dependency, bound or historical verdict changes.

## 1. Baseline

- `main` after PR #132 (M3) merged.
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18; `Cargo.lock`
  unchanged by Stage44.
- Tag `v1.0.0-rc.2` is frozen and unchanged. The fix is **not** in the
  rc.2 archive.
- S3-P2 and S2-P4 stay EXCLUDED. Stage38 stays CLOSED.

## 2. Milestones

| Step | PR | Result | Record |
|---|---|---|---|
| Survey | #129 | H1 QUIC endpoint IPv4-only, H3 share per IPv6 address, H4 CLI bind. Recommended V1 + V2 + V3 + V4 tool; first-address dialling recorded only; sandbox only. S44-D0–D5 as recommended | [survey](STAGE44_IPV6_SURVEY.md) |
| M1 test | #130 | H1 and H3 CONFIRMED (silent QUIC failure, `rejected` status); H2 and H5 hold; H4 not a defect (owner kept V3 as a spelling cleanup) | [M1](STAGE44_M1_IPV6_REPRODUCTION.md) |
| M2 fix | #131 | M2-PASS (`716ec8a`): QUIC endpoint per destination family, /64 share, CLI bind bracketing; 428 tests; local CI 13/13 | [M2](STAGE44_M2_IPV6_FIX.md) |
| M3 regression | #132 | PASS: IPv6 scenarios ×3; IPv4 Stage43 and Stage42 flood scenarios unchanged; Stage32 S2 P0/P1, S3 P0, S4 P0, S7 P0 3/3 each; routed-Fang gate 12/12 | [M3](STAGE44_M3_REGRESSION.md) |
| M4 | this PR | `KNOWN_LIMITATIONS.md` updated; closure | — |

## 3. What remains open

- **Real IPv6 networks.** IPv6 was run only on unique-local addresses in
  a namespace. Router advertisements, path MTU, real WAN paths and the
  Stage32 impairments over IPv6 were not run. WAN testing is not
  authorised.
- **Dialling** (S44-D4, recorded only): a hostname peer address uses its
  first resolved address, with no fallback between addresses or families.
- **/64 share heuristic** (V2): larger prefixes evade it, and hosts
  sharing a /64 share its 16 slots.
- **Diagnostics:** a QUIC dial failure is not logged. Discovery and sync
  report a local connection error as `rejected`, the status a trust
  refusal also gets. Out of Stage44's scope.
- **Not covered:** failover Fangs over IPv6. They use the same QUIC
  endpoint and TCP paths as the scenarios run.
- From Stage43: the active loopback flood (F2), the shared peer quota
  (F5), the operator ceiling per Fang (F4) and half-close. From earlier
  stages: the initiator's stream windows (Stage41; the send window is the
  Stage34 surface), the routed-QUIC regression root cause (Stage34), and
  the real-WAN relay-to-destination check (S39-D5).
- **Tooling:** `tools/stage44/ipv6.py` keeps the Stage42 50 MB log
  guard.

**Not authorised:** tokio promotion, rc.3, release certification, WAN
testing.
