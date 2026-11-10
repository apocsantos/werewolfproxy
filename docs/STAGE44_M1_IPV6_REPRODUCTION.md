# Stage44 M1 — IPv6, reproduced

**M1: H1 and H3 CONFIRMED; H2 and H5 hold; H4 NOT reproduced.**

- **H1, QUIC to an IPv6 peer fails everywhere.** QUIC Fang, the QUIC hop
  of a routed Fang, discovery and governance sync over QUIC all fail
  against an IPv6 peer. The failure is silent: the Fang reports
  `active`, every session is reset, and neither daemon logs a line.
  Discovery and sync report the peer as `rejected`, the status a trust
  refusal also gets.
- **H2, everything over TCP works.** Encrypted-TCP and plain-TCP Fangs
  (echo and a 1 MiB full-duplex transfer by SHA-256), discovery and sync
  over TCP, and a routed Fang over a TCP hop, with IPv6 peers, an IPv6
  target granted by Target Policy, and Fangs bound to `[::1]`.
- **H3, the Stage43 share is per address.** On a Fang bound to an IPv6
  address, four addresses in one /64 hold 16 slots each, all 64; a client
  from another /64 is reset.
- **H5, a dual-stack receiver works.** A receiver on `[::]` serves an IPv4
  peer over QUIC and TCP and an IPv6 peer over TCP (its QUIC is H1).
- **H4, not a defect.** `service connect --bind ::1` opens a working
  Fang; the daemon resolves `::1:PORT` by splitting at the last colon. The
  Fang is recorded with that unbracketed spelling (`::1:55867`), where
  `--bind [::1]` gives `[::1]:57665`.

No production source changed.

## 1. Baseline

- `main` `1222c1f` plus the survey branch (`6f4f247`, docs only). Lab
  debug `werewolfd`
  `9649ad08c6f87bb0297c551d3ae262b16e276db119bd2faad3b7feeab9b10ee5`,
  the Stage43 M2/M3 binary (no source change since).
- quinn 0.11.12, quinn-proto 0.11.18, tokio 1.52.3.
- Owner decisions S44-D0–D5 as recommended (V1 per-family QUIC endpoint,
  V2 /64 share, V3 CLI bind, first-address dialling recorded only,
  sandbox only).

## 2. Method

`tools/stage44/ipv6.py`, run 2026-11-10 16:21–16:22 UTC, once per
scenario, each in a fresh lab. Every scenario runs in a user, mount and
network namespace with a private root and a private 512 MB `/tmp` (the
Stage43 sandbox). The namespace's `lo` carries `fd43::1` (A), `fd43::2`
(B), `fd43::3` (C), flood sources `fd43::10`–`fd43::13`, and a client in
another /64, `fd43:0:0:1::2`. Each wolf listens (TCP and QUIC, one port)
on its own address; Pack entries are `quic://[fd43::N]:PORT`. Echo targets
have no idle timeout and listen on the receiver's IPv6 address; the
receiver grants exactly that target. The 50 MB log guard applies. The
results keep each daemon's last log lines, since the lab's `/tmp` goes
with the namespace.

Results: `target/stage44-m1/m1-ipv6.json` and `m1-ipv6.log`.

## 3. Results

| Scenario | What | Result |
|---|---|---|
| I1 `fang:quic` | B → A QUIC Fang, bound `[::1]` | Fang opens, `active`; echo **reset**; 1 MiB **0 bytes**. No log line on A or B |
| I2 `fang:tcp` | encrypted TCP | echo PASS; 1 MiB PASS (SHA-256) |
| I2 `fang:tcp-plain` | plain TCP | echo PASS; 1 MiB PASS |
| I3 `discovery:quic` | A refreshes B over QUIC | wolf-b **`rejected`**, nothing cached |
| I3 `discovery:tcp` | over TCP | wolf-b `ok`, 1 record; cached `quic://[fd43::2]:P`, `tcp://[fd43::2]:P` |
| I3 `sync:quic` | B syncs one admission from A over QUIC | wolf-a **`rejected`**, nothing applied, head 0 |
| I3 `sync:tcp` | over TCP | wolf-a `ok`, applied [1], head 1 |
| I4 `routed:quic` | A → relay B (QUIC hop) → C, all IPv6 | `tcp via wolf-b (quic)`; echo **reset**; 1 MiB **0 bytes** |
| I4 `routed:tcp` | TCP hop (`route set --hop tcp`) | `tcp via wolf-b (tcp)`; echo PASS; 1 MiB PASS |
| I5 `dualstack` | A on `[::]`; B IPv6, C IPv4 | IPv4 QUIC PASS, IPv4 TCP PASS, IPv6 TCP PASS, IPv6 QUIC **reset** (H1) |
| I6 `share:tcp` | Fang on `[fd43::2]`; 4 sources in one /64 | held 16/16/16/16 = **64**; client from another /64 **reset**; after release PASS |
| I6 `share:tcp-plain` | same | same |
| I7 `cli` | `--bind ::1` / `--bind [::1]` | both open and echo; local spelled `::1:P` / `[::1]:P` |

Daemons stayed up in every scenario; the log guard never triggered; host
`/tmp` 5%. I6 used TCP kinds: a QUIC Fang to an IPv6 peer would fail its
sessions (H1), though admission, which I6 tests, is the same for every
kind.

## 4. Notes

- **Cause of H1, as read in the survey.** `client_endpoint()` binds
  `0.0.0.0:0`; quinn refuses an IPv6 destination on an IPv4 endpoint
  before sending anything (`InvalidRemoteAddress`). M1 confirms the
  behaviour, not the line: the throwaway-instrumentation step was not
  needed, because every QUIC path fails and every TCP path works with the
  same peers, addresses and targets.
- **Silent failure.** The QUIC Fang's session error is not logged, and
  discovery and sync map the local connect error to `rejected`. V1 removes
  the cause; this survey did not propose changing the logging or status
  mapping, so they are recorded here only.
- **H4.** Not a defect in function. If V3 stays in M2 it is a spelling
  cleanup (the stored local address becomes `[::1]:P`), not a fix.
- **Not covered:** a real IPv6 network (router advertisements, path MTU,
  a WAN); hostnames resolving to AAAA; failover Fangs (they use the same
  QUIC client endpoint and TCP paths).

## 5. Conclusion

H1 and H3 are CONFIRMED. M2 proceeds with V1 (QUIC client endpoint per
destination family) and V2 (IPv6 sources keyed by /64). V3 is reduced to
a spelling cleanup; owner to confirm whether it stays in M2.
