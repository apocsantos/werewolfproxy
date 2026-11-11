# Stage46 M3 — Regression

**M3: PASS.** The Stage46 matrix gives the M2 results in all three
rounds, identically. The Stage45 visibility scenarios, the Stage44 IPv6
scenarios, the Stage42 flood scenarios, the Stage32 cells and every
`ci-local` gate hold, and `localhost` now works over QUIC. Every historical
verdict holds. One observation (§5): under descriptor exhaustion an
encrypted-TCP Fang's daemon uses more CPU than before M2 (0.8–8.8% of a
core against 1.2–1.8%), bounded and recovering. The owner chose to record
it as a limitation in M4.

## 1. Build

- Tested commit `4dee787` (M2, PR #142, on `main` `e897b31`, which has
  the rustix 1.1.6 fix, PR #140).
- Lab debug `werewolfd`
  `e294de2b358308668e63c633efb3171e8c8666e98de9f1f2afbc901605038ff4`.
  It differs from the M2 lab binary (`84467020…`) only by rustix 1.1.6.
  Every lab result file records it.
- Run 2026-11-11 12:41–13:26 UTC, then the §5 A/B runs. `/tmp` stayed at
  8% (task output of this session, not the lab); the log guard never
  triggered; the daemons stayed up in every run.
- No source or tooling change in this PR.

Results in `target/stage46-m3/` (`meta.txt` has the order and times) and
`target/stage46-ab/`; chaos in
`target/stage32-chaos/results/20261111T131035Z-network`; `ci-local` in
`target/ci-local/20261111T132000Z`.

## 2. Stage46 matrix (`tools/stage46/dial.py`), three rounds

40 scenarios, 5 clients each. Rounds 1–3 match each other and M2 to within
0.06 s on every client time:

| Path | `fast` | `silent` | `family` | `dead` |
|---|---|---|---|---|
| QUIC | echo, 0.31 | echo, 0.31 | echo, 0.31 | reset, 5.0; `dial` |
| encrypted TCP | echo, 0.01 | echo, 0.29 | echo, 0.02 | reset, 10.0 (F1 retry); `dial` |
| plain TCP | echo, 0.00 | echo, 0.25–0.26 | echo, 0.00 | reset, 5.0; `target` |
| failover (QUIC provider by name) | echo, 0.31–0.32 | echo, 0.31 | echo, 0.30–0.32 | echo via C, 5.06 |
| routed, QUIC hop | echo, 0.35–0.36 | echo, 0.35–0.36 | echo, 0.35–0.36 | reset, 5.0; `dial` |
| routed, TCP hop | echo, 0.08–0.15 | echo, 0.38–0.40 | echo, 0.13–0.14 | reset, 5.0; `dial` |
| discovery, QUIC / TCP | `ok`, 0.3 / 0.0 | `ok`, 0.3 / 0.3 | `ok`, 0.3 / 0.0 | `unreachable`, 5.0 |
| sync, QUIC / TCP | `ok`, 0.3–0.4 / 0.0 | `ok`, 0.3 / 0.3 | `ok`, 0.3–0.4 / 0.0 | `unreachable`, 5.0 |

`sessions_failed` is 0 wherever every client echoed and 6 where 5 failed
(the readiness probe counts, Stage45).

## 3. Stage45 visibility (`tools/stage45/visibility.py`), one round

| Scenario | Stage45 M3 | Stage46 M3 |
|---|---|---|
| `grant:*` | 1 line, `handshake`; 11 failed | same |
| `dial:quic`, `dial:routed` | 5 lines (538 bytes), `dial`; 50 s | same |
| `dial:tcp` | 1 line, `dial` | same |
| `dial:failover` | 10 lines (1,317 bytes), `dial`; 100 s | same |
| `dial:tcp-plain` | 1 line, `target` | same |
| `flood:tcp-plain`, 500 | 1 line, 119 bytes; 501 failed | same |
| `hostname:quic` (`localhost`, host resolver: `::1` first) | reset; 5 lines, `dial` | **echo 10/10**, 0.3 s each, 0 lines, 0 failed |
| `hostname:tcp` (`localhost`) | echo | echo |
| `hostname:quic`, `hostname:tcp` (`wolf-b.stage45`, sandbox) | echo 10/10 | echo 10/10 |

## 4. Stage44 IPv6 (`tools/stage44/ipv6.py`), one round

All 13 scenarios as in Stage44 M3: QUIC, encrypted-TCP and plain-TCP Fangs
echo and carry 1 MiB both ways over IPv6; discovery and sync `ok` over
both transports; routed over QUIC and TCP hops; the dual-stack receiver on
all four combinations; the /64 share holds a flood to 16 with the
legitimate client served during and after; the CLI bind spellings.

## 5. Stage42 flood scenarios (`tools/stage42/fang_flood.py`), one round

| Scenario | Stage46 M3 | Stage45 M3 |
|---|---|---|
| `fd:*`, `nofile` 256 | listening throughout; 70 / 145 / 145 descriptors; 0% CPU; 17–18 after release; legit PASS | 68 / 129 / 145; same otherwise |
| `quota:*` | echoed 64 / 61 / 64; legit during FAIL / PASS / FAIL; after PASS | same |
| `fd:quic`, `nofile` 64 | 64 descriptors; 1.6% CPU; 295 bytes; recovered | 64; 1.4%; 295 bytes |
| `fd:tcp`, `nofile` 64 | 43 descriptors; **4.8% CPU; 1,593 bytes**; recovered, legit PASS | 63; 1.6%; 935 bytes |

**`fd:tcp` at `nofile` 64, A/B.** The flood was run five more times on
this build and five times on `e897b31` (`main` just before M2, built in a
temporary worktree since removed):

| Build | CPU while flooded | B's log | Flood |
|---|---|---|---|
| before M2 (5 runs) | 1.2–1.8% | 935 bytes every run | 37.2–37.8 s |
| M2 (6 runs, incl. the round above) | 0.8, 2.6, 4.8, 6.6, 6.8, 8.8% | 1,123–1,828 bytes | 31.2–37.8 s |

- **Log:** bounded, as designed. Descriptor exhaustion is now `dial`, so
  the receiver's refusals in the same flood get their own `handshake`
  line instead of falling inside one class's suppressed count (M2 §3):
  two lines per 10 s instead of one, each with its count.
- **CPU:** higher and more variable on M2, from a single 5 s sample per
  run. The cause was not investigated; the owner chose to record it. It
  is far from the Stage42 defect it guards against (a 103% spin), it ends
  with the flood (0% after release), and the Fang recovers and serves a
  legitimate client in every run.

## 6. Stage32 cells (`chaos.py network`), because every dial path changed

| Cell | M3 | Historical | Detail (runs 1–3) | Stage45 M3 |
|---|---|---|---|---|
| S2-P0 | PASS 3/3 | PASS | 0.536–0.541 s, exact match | 0.50–0.53 s |
| S2-P1 | PASS 3/3 | PASS | 0.537–0.584 s | — (Stage44: 0.51–0.56 s) |
| S3-P0 | PASS 3/3 | PASS | QUIC hop 0.79–0.85 s; TCP hop 1.15–1.21 s | 0.78–0.84 s; 1.16–1.21 s |
| S4-P0 | PASS 3/3 | PASS | 50/50 per transport; max QUIC 0.014–0.016 s, TCP 0.007–0.008 s | 0.014 s |
| S7-P0 | PASS 3/3 | PASS | 10/10 selections QUIC | same |

The chaos paths dial literal addresses, so they exercise the new dialer
in its single-address case.

## 7. Gates (`scripts/ci-local.sh` full)

**13 of 13 PASS**, including the audit (rustix 1.1.6), the acceptance lab
and the discovery, sync, routed-Fang, provider and service gates.

## 8. Conclusion

All M3 criteria hold. M4 is the `KNOWN_LIMITATIONS` update and the
closure. To record there:

- the `fd:tcp` CPU observation (§5), uninvestigated;
- an encrypted-TCP Fang where no address answers still takes 10 s per
  client (the Stage32 F1 retry repeats the whole race);
- QUIC resolves a name once, at open: a later DNS change needs a reopen;
  TCP and discovery resolve at each dial;
- a dead QUIC peer still costs each client the 5 s window;
- the receiver's refusal reasons stay indistinguishable on the initiator,
  by design;
- the dialer was run only in the namespace sandbox, on loopback and
  unique-local addresses, never against a real dual-stack network.
