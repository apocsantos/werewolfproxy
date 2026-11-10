# Stage45 M3 — Regression

**M3: PASS.** The M1 scenarios give the M2 results in all three rounds,
identically. The Stage42 flood scenarios, the Stage32 QUIC cells and every
`ci-local` gate hold. Every historical verdict holds. One labelling flaw
was found (§3): a setup failure caused by descriptor exhaustion is
classed `handshake` rather than `dial`. It changes no behaviour and is
recorded for M4.

## 1. Build

- Tested commit `35f0e3f` (M2 branch head, PR #136, on `main`
  `600852f`).
- Lab debug `werewolfd`
  `405131f44c0f931c215eec4413def39cdf520c9bbc6511de9afeff7c8f55060a`,
  the M2 binary. Every lab result file records that hash.
- Run 2026-11-10 21:34–22:09 UTC. `/tmp` stayed at 5%; the log guard never
  triggered; the daemons stayed up in every run.
- No source or tooling change in this PR.

Results:

- `target/stage45-m3/`: `r{1,2,3}-visibility.json`,
  `r{1,2,3}-hostname4.json`, `s42-256.json`, `s42-64.json`, logs,
  `chaos.log`, `ci-local.log` and `meta.txt`;
- chaos: `target/stage32-chaos/results/20261110T215537Z-network`;
- `ci-local`: `target/ci-local/20261110T220312Z`.

## 2. M1 scenarios (`tools/stage45/visibility.py`), three rounds

10 clients each (flood 500). `sessions_failed` counts the lab's readiness
probe too (11 for 10 clients, 501 for 500).

| Scenario | M1 (`main` before M2) | M3 r1 / r2 / r3 |
|---|---|---|
| `grant:quic`, `grant:tcp`, `grant:routed`, `grant:failover` | reset; 0 lines | reset; 1 line, `handshake`; `sessions_failed` 11 ×3 |
| `dial:quic`, `dial:routed` | reset; 0 lines; 50 s | reset; 5 lines (538 bytes), `dial`; 50 s ×3 |
| `dial:tcp` | reset; 0 lines | reset; 1 line, `dial` ×3 |
| `dial:failover` | reset; 0 lines; 100 s | reset; 10 lines (1,317 bytes), `dial`; 100 s ×3 |
| `dial:tcp-plain` | reset; 11 lines | reset; 1 line, `target` ×3 |
| `flood:tcp-plain`, 500 | 501 lines, 52,605 bytes | **1 line, 119 bytes**; `sessions_failed` 501 ×3 |
| `hostname:quic` (`localhost`) | reset; `bad server addr` per connection | reset; 5 lines, `dial`, with Fang id ×3 (first-address limitation, M2 §3) |
| `hostname:tcp` (`localhost`) | echo | echo ×3 |
| `hostname:quic`, `hostname:tcp` (`wolf-b.stage45`, `--sandbox-hosts`) | — | echo 10/10, 0 lines, 0 failed ×3 |

`fang active` showed `state` `Active` with the class above in every case.

## 3. Stage42 flood scenarios, one round (`tools/stage42/fang_flood.py`)

B runs the Fang; A is the receiver.

| Scenario | M3 | Stage44 M3 |
|---|---|---|
| `fd:*`, `nofile` 256 | listening throughout; 68 / 129 / 145 descriptors; 0% CPU; 17–18 after release; legit PASS | 82 / 145 / 145; same otherwise |
| `fd:quic`, `fd:tcp`, `nofile` 64 | 64 / 63 descriptors; 1.4% / 1.6% CPU; **295 / 935 bytes** of B's log; recovered, legit PASS | 64 / 63; 1.8% / 2.0%; 188 / 428 bytes |
| `quota:*` | echoed 64 / 61 / 64; legit during FAIL / PASS / FAIL; after PASS; 17–18 descriptors after release | 64 / 63 / 64; same pattern |

Descriptor peaks are at or below Stage44's. The quota pattern is the
Stage43 limit, unchanged.

**B's log grows more under descriptor exhaustion, as designed.** A rerun
of `fd:tcp` at `nofile` 64 with its log kept shows the difference: next to
the Stage42 `accept failed, retrying` line (one per 10 s), B now logs
`session setup failed`, also one per 10 s, with the suppressed count
(106–109 per line). 935 bytes in 38.8 s is 4 lines of each kind. Bounded,
not a regression.

**Labelling flaw (found here).** Those lines read
`session setup failed (handshake): Too many open files (os error 24)`.
The outbound socket could not be created, so no connection was made and
the class should be `dial`. The classifier goes by error kind, and
`EMFILE` is not one of the dial kinds (the set discovery reports as
`unreachable`), so it falls to `handshake`. The same holds for any local
resource error on the dial (`ENFILE`, `ENOBUFS`, `ENOMEM`). Only the class
label in the log line and in `last_failure` is affected; counting, the
limiter and every session outcome are correct, and the accept line next to
it names the cause. Discovery reports the same error `rejected`, as before
Stage45. Recorded for M4.

## 4. Stage32 cells (`chaos.py network`), because the QUIC dial path changed

| Cell | M3 | Historical | Detail (runs 1–3) | Stage44 M3 |
|---|---|---|---|---|
| S2-P0 | PASS 3/3 | PASS | 0.50–0.53 s, exact match | 0.53–0.54 s |
| S3-P0 | PASS 3/3 | PASS | QUIC hop 0.78–0.84 s; TCP hop 1.16–1.21 s | 0.78–0.85 s; 1.16–1.23 s |
| S4-P0 | PASS 3/3 | PASS | 50/50 per transport; max QUIC 0.014 s | 0.015 s |
| S7-P0 | PASS 3/3 | PASS | 10/10 selections QUIC | same |

## 5. Gates (`scripts/ci-local.sh` full)

**13 of 13 PASS**: python-unit, audit, acceptance, and the activation,
auto-sync, death, discovery, governance, provider, routed-Fang, service,
service-discovery and sync gates. The discovery, sync and routed-Fang gates
cover S1a: their real refusals still report `rejected`.

## 6. Conclusion

All M3 criteria hold. M4 is the `KNOWN_LIMITATIONS` update and the
closure. To record there:

- a QUIC Fang (and discovery and sync) dials a hostname's first address
  only; `localhost` → `::1` first fails against an IPv4-only listener,
  now visibly;
- setup failures from local resource exhaustion are labelled `handshake`,
  not `dial` (§3);
- the receiver's refusal reasons stay indistinguishable on the initiator,
  by design;
- a dead QUIC peer still costs each client the 5 s read window.
