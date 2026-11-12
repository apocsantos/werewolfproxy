# Stage49 — closure record

**Status: delivery correction awaiting regression (2026-11-12).**
A QUIC dial to a host with nothing listening now fails as soon as a
separate connected UDP probe receives a port refusal. Direct and routed
Fangs reset in 0.00 s, discovery and governance sync report `unreachable`
in 0.01 s, and failover reaches a live provider behind one or two refused
providers in 0.02–0.03 s. A hostname's refused first address no longer
holds the next address for the 250 ms stagger. Silent peers and TCP
retain their existing behaviour.

This record includes the baseline local validation, corrected timeout
fixtures and a bounded rebind preventing a UDP probe from using its own
destination port. Fresh regression is required for the production
correction; transport bounds and probe datagram/socket ceilings stay
unchanged.

## 1. Baseline

- M2 merged to `main` as PR #157 (`6ac6594`). Tested commit
  `267385fd8ac54963c20f13501ac09be8f4684422`, on
  `stage49/m3-regression`, with the M2 refusal-probe fix. M3 and M4 add
  documentation, correct the silent-UDP fixtures (`d280f5b`) and prevent
  a probe's source port from matching its destination. The [M3 delivery
  follow-up](STAGE49_M3_REGRESSION.md) records the failures and correction.
- Lab debug daemon hash `c484a543…`; the full hash and the separate
  Stage32 campaign build provenance are in the [M3 record](STAGE49_M3_REGRESSION.md).
- tokio 1.52.3, quinn 0.11.12, quinn-proto 0.11.18, quinn-udp 0.5.14,
  rustix 1.1.6. No dependency change in Stage49.
- Tag `v1.0.0-rc.2` and its archive remain frozen. The fix is not in the
  rc.2 archive. S3-P2 and S2-P4 stay EXCLUDED; Stage38 stays CLOSED.
- S49-D0–D1: Stage49 scoped as approved in the M1 record; R1 is the
  accepted refusal probe, including its new one-byte datagram per QUIC
  address attempt. No Werewolf message, handshake, trust, Target Policy
  or receiver change.

## 2. Milestones

| Step | Result | Record |
|---|---|---|
| Survey (#155) | P1: every QUIC path costs 5 s per refused peer; P2: a refused first address waits for the stagger. R1 proposes a separate connected UDP probe; R2/R3 rejected | [survey](STAGE49_QUIC_REFUSED_SURVEY.md) |
| M1 (#156) | P1/P2 CONFIRMED. Direct/routed refusal 5.0 s, discovery/sync 5.01 s, failover 5.05–5.06 s / 10.05 s; `fast` 0.30–0.36 s. Live receiver discards 10,000 one-byte datagrams with no log or session disruption | [M1](STAGE49_M1_QUIC_REFUSED_REPRODUCTION.md) |
| M2 (#157, `267385f`) | PASS: each QUIC dial races its probe; refusal ends it early, other probe errors leave the dial running. All paths meet the timing criteria; three new unit tests, 459 tests total and 13/13 local gates passed | [M2](STAGE49_M2_QUIC_REFUSED_FIX.md) |
| M3 | PASS: Stage49 tool ×3, Stage46 matrix ×1, Stage48 matrix ×1, QUIC descriptor/quota floods, Stage32 S2/S3/S4/S7 P0 3/3 each, 13/13 local gates; timeout-test fixtures corrected during delivery | [M3](STAGE49_M3_REGRESSION.md) |
| M4 | Limitations updated; local engineering closure | [known limitations](../KNOWN_LIMITATIONS.md) |

The M3 scenario results survived the host crash and were checked against
their expected outcomes and binary hashes. The interrupted local-CI run
had empty logs and supplies no verdict. The completed recovery run is
`target/ci-local/20261112T091031Z`, with all 13 checks passing; the M3
record distinguishes it from the failed sandbox recovery attempt.

## 3. Remaining limits

- **Filtered or absent ICMP:** the probe sees no refusal, so a refused
  peer still costs the 5 s window. Silent peers also retain that window.
- **Forged ICMP:** a port-unreachable error forged for the probe's source
  port can fail an attempt early, causing a client reset or provider
  failover. The probe does not authenticate the error.
- **Traffic and resources:** every QUIC address attempt sends one extra
  one-byte UDP datagram and holds one extra socket until the attempt
  ends. A probe that cannot open, or has another error, leaves the QUIC
  dial running. The descriptor-exhaustion runs recovered to 18
  descriptors and served new sessions after release, with no sustained
  spin; this does not guarantee service during resource exhaustion.
- **Local validation:** real-WAN ICMP delivery/filtering and the
  dialer's real dual-stack behaviour remain untested.
- **Stage48:** direct encrypted-TCP Fangs retain the Stage32 F1 retry
  and the 10 s reset for an unanswered peer (S48-D2). Failover skips a
  provider after its 5 s connect window, giving up a second connect
  chance under heavy loss.
- Earlier open items remain as recorded in the
  [Stage48 closure](STAGE48_CLOSURE.md): hostname-race CPU overhead,
  open-time QUIC name resolution, receiver refusal reasons hidden from
  the initiator, real IPv6 networks and the /64 share heuristic, active
  loopback floods and shared quotas, half-close, initiator stream
  windows, the routed-QUIC loss regression and the real-WAN
  relay-to-destination check.

Tokio promotion, rc.3, release certification and WAN testing remain
outside this stage's authorised scope.
