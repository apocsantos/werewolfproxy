# Stage48 — closure record

**Status: Stage48 CLOSED.** Failover no longer redials an encrypted-TCP
provider that did not answer. A silent TCP provider now costs a local
connection about 5 s before the next provider, instead of 10 s, and two
of them no longer use up the 15 s selection window: a live third provider
serves the session at about 10 s, as with QUIC providers. The direct
encrypted-TCP Fang keeps every Stage32 F1 retry and still takes 10 s on a
dead peer, by owner decision.

This record is documentation only: no production source, harness,
dependency, bound or historical verdict changes.

## 1. Baseline

- `main` after PR #153 (M3) merged.
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18, rustix 1.1.6. No
  `Cargo.lock` change in Stage48.
- Tag `v1.0.0-rc.2` is frozen and unchanged. The fix is **not** in the
  rc.2 archive.
- S3-P2 and S2-P4 stay EXCLUDED. Stage38 stays CLOSED.
- No wire, trust, Target Policy or receiver change: the receiver never
  sees the attempt that is no longer made.

## 2. Milestones

| Step | PR | Result | Record |
|---|---|---|---|
| Survey | #150 | P1 failover delayed 10 s per silent TCP provider, P2 a third provider unreached (code reading), P3 direct Fang 10 s. Recommended O1 (failover only) and P3 recorded. S48-D0–D2 approved as recommended | [survey](STAGE48_TCP_SETUP_RETRY_SURVEY.md) |
| M1 test | #151 | P1–P3 CONFIRMED with `tools/stage48/retry.py` (Stage46 sandbox, a fourth daemon): one silent TCP provider 10.04–10.06 s; two, every client reset at 15.0 s; direct 10.0 s; QUIC 5 / 10 / 5 s | [M1](STAGE48_M1_RETRY_REPRODUCTION.md) |
| M2 fix | #152 | M2-PASS (`3c7bbff`): `SetupRetry`; failover's leg takes a connect timeout as final. One silent TCP provider 5.02–5.04 s; two, the third at 10.03–10.05 s. 2 new tests; 456 tests; local CI 13/13 | [M2](STAGE48_M2_RETRY_FIX.md) |
| M3 regression | #153 | PASS: tool ×3 (5.03–5.06 s; 10.03–10.05 s); Stage46 matrix unchanged; Stage32 S2, S4, S7 P0 3/3; local CI 13/13 | [M3](STAGE48_M3_REGRESSION.md) |
| M4 | this PR | `KNOWN_LIMITATIONS.md` updated; closure | — |

## 3. What remains open

- **Direct encrypted-TCP Fang, dead peer (P3):** each client is reset at
  10 s (QUIC, plain TCP: 5 s). Changing it would alter the owner-set F1
  boundary on the direct path and needs the Stage32 S1-P2 loss rerun.
- **Failover under heavy loss:** a provider whose connect gets no answer
  for the whole 5 s window is passed over instead of dialled again; F1's
  validation saw no such case in 600 sessions under 10% loss.
- **A QUIC peer with nothing listening** costs 5 s, like a silent one:
  QUIC does not act on ICMP port unreachable (seen in M1; the Stage45
  item).
- From Stage47: why the race's task machinery cost about 0.27 ms per dial
  under descriptor exhaustion; Stage43–46 flood CPU figures are indicative
  only. From Stage46: QUIC resolves a name only at open; the dialer was
  run only in the namespace sandbox. From Stage45: refusal reasons stay
  hidden from the initiator, by design. From Stage44: real IPv6 networks,
  the /64 share heuristic, failover Fangs over IPv6. From Stage43: the
  active loopback flood (F2), the shared peer quota (F5), the operator
  ceiling per Fang (F4) and half-close. From earlier stages: the
  initiator's stream windows (Stage41; the send window is the Stage34
  surface), the routed-QUIC regression root cause (Stage34), and the
  real-WAN relay-to-destination check (S39-D5).

**Not authorised:** tokio promotion, rc.3, release certification, WAN
testing.
