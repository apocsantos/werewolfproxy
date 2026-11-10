# Stage45 — closure record

**Status: Stage45 CLOSED.** A Fang whose sessions fail to set up is now
visible. Every Fang kind logs the failure at a bounded rate with its
class. `fang active` counts the failures. Discovery and sync report a
local dial failure as `unreachable`. A QUIC Fang resolves a hostname peer
when it opens.

This record is documentation only: no production source, harness,
dependency, bound or historical verdict changes.

## 1. Baseline

- `main` after PR #137 (M3) merged.
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18; `Cargo.lock`
  unchanged by Stage45.
- Tag `v1.0.0-rc.2` is frozen and unchanged. The fix is **not** in the
  rc.2 archive.
- S3-P2 and S2-P4 stay EXCLUDED. Stage38 stays CLOSED.
- No wire, trust, Target Policy or receiver change.

## 2. Milestones

| Step | PR | Result | Record |
|---|---|---|---|
| Survey | #134 | P1 silent setup failures, P2 unbounded plain-TCP lines, P3 `fang active` blind, P4 `rejected` for a local dial error, P5 QUIC Fang to a hostname peer. Recommended L1, the limiter for plain TCP, S1a, C1, H1. S45-D0–D5 approved as recommended | [survey](STAGE45_FANG_FAILURE_VISIBILITY_SURVEY.md) |
| M1 test | #135 | P1–P5 CONFIRMED | [M1](STAGE45_M1_VISIBILITY_REPRODUCTION.md) |
| M2 fix | #136 | M2-PASS (`35f0e3f`): per-Fang `FangHealth` with a rate-limited, classed setup-failure line; plain TCP on the limiter; `quic_fang::dial` shared with discovery and sync (S1a); `sessions_failed` and `last_failure` in `fang list`; hostname resolved at open (first address). 437 tests; local CI 13/13. Owner kept H1 with the first address and recorded `localhost` → `::1` as a limitation | [M2](STAGE45_M2_VISIBILITY_FIX.md) |
| M3 regression | #137 | PASS: M1 scenarios ×3, identical; IPv4-only name echoes 10/10 over QUIC and TCP; Stage42 flood scenarios hold, B's log bounded; Stage32 S2, S3, S4, S7 P0 3/3 each; local CI 13/13. Found the `EMFILE` labelling flaw | [M3](STAGE45_M3_REGRESSION.md) |
| M4 | this PR | `KNOWN_LIMITATIONS.md` updated; closure | — |

## 3. What remains open

- **First-address dialling** (S44-D4, kept by H1): QUIC Fangs, discovery
  and sync dial a hostname's first resolved address, with no fallback.
  `localhost` → `::1` first fails against an IPv4-only listener; it is now
  logged as `dial` and counted.
- **Labelling:** a setup failure from a local resource limit (`EMFILE`,
  `ENFILE`, `ENOBUFS`, `ENOMEM`) is labelled `handshake`, not `dial`, and
  discovery reports it `rejected`. Label only; counting, the limiter and
  session outcomes are correct.
- **Refusal reasons:** the initiator cannot tell no grant, Silver and a
  key mismatch apart. By design.
- **Dead QUIC peer:** each client still waits the 5 s read window before
  its reset.
- From Stage44: real IPv6 networks, the /64 share heuristic, failover
  Fangs over IPv6. From Stage43: the active loopback flood (F2), the
  shared peer quota (F5), the operator ceiling per Fang (F4) and
  half-close. From earlier stages: the initiator's stream windows
  (Stage41; the send window is the Stage34 surface), the routed-QUIC
  regression root cause (Stage34), and the real-WAN relay-to-destination
  check (S39-D5).
- **Tooling:** `tools/stage45/visibility.py` keeps the Stage42 50 MB log
  guard. `--sandbox-hosts` uses an unprivileged namespace with a private
  `/etc/hosts`; the host is not touched.

**Not authorised:** tokio promotion, rc.3, release certification, WAN
testing.
