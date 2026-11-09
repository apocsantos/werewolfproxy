# Stage41 M3 — regression of the QUIC initiator bounds

**M3: PASS.** The M2 change introduced no regression in the authorised
scope. All 10 cells and all 30 runs passed, and every cell keeps its
historical verdict. S3-P2 and S2-P4 were not run and were not criteria
(EXCLUDED, Stage34 and F3).

No production code, harness rule, profile, bound or historical verdict
changed.

## 1. Build and baseline

| | |
|---|---|
| Commit | `d632f960966afe08bec46258ca90a6344cf8d8e7` (PR #117 head: M2 fix `5449261` and its record, on `main` `b003958`) |
| `werewolfd` | `fe5b6d57d9be1540f46c9cdc1a54fbb1a5e6aa1908482bf0f21fbe07e7300584` (certification build, release, `--locked`, built by the harness) |
| Unchanged | tokio 1.52.3, quinn-proto 0.11.18, `Cargo.lock` |
| Host | DarkSide, Linux 6.12.115+deb13.1-amd64, rustc 1.96.0, iproute2 6.15.0; no sysctl changed |
| Tree | clean for tracked files (only untracked files present) and not edited during the run |
| Command | `chaos.py network --cells S2:P0,S2:P1,S2:P2,S2:P5,S3:P0,S3:P1,S4:P0,S4:P2,S5:P0,S7:P0` (three seeds per cell, unchanged Stage32 rules) |
| Results | `target/stage32-chaos/results/20261109T162150Z-network` (16:23–16:48 UTC, 2026-11-09) |

## 2. Scope

These are the cells whose initiator uses `client_endpoint`, from the
survey §8 and owner decision S41-D4: direct QUIC (S2), the routed QUIC
hop (S3), connection setup (S4), discovery (S5) and failover selection
with QUIC impaired (S7).

## 3. Results

| Cell | M3 | Historical | Detail (runs 1–3) |
|---|---|---|---|
| S2-P0 | PASS 3/3 | PASS | 0.53–0.54 s, exact match |
| S2-P1 | PASS 3/3 | PASS | 0.55–0.64 s |
| S2-P2 | PASS 3/3 | PASS | 6.6–11.1 s |
| S2-P5 | PASS 3/3 | PASS | 58.0–58.9 s |
| S3-P0 | PASS 3/3 | PASS | QUIC hop 0.79–0.87 s; TCP hop 1.15–1.19 s |
| S3-P1 | PASS 3/3 | PASS | QUIC hop 0.89–0.99 s; TCP hop 2.6–3.4 s |
| S4-P0 | PASS 3/3 | PASS | 50/50 opens per transport; max QUIC 0.015 s |
| S4-P2 | PASS 3/3 | PASS | 50/50 per transport; max QUIC 1.1–3.1 s |
| S5-P0 | PASS 3/3 | PASS | manual sync applied [1–4]; automatic [3] in 1.5 s |
| S7-P0 | PASS 3/3 | PASS | 10/10 selections QUIC |

Every transfer matched by SHA-256 in both directions. The harness's own
leak and liveness checks passed in every run.

Not run, and not criteria: S2-P4 (EXCLUDED, F3) and S3-P2 (EXCLUDED,
Stage34). The survey's rule stands: the M2 change does not touch the
send window, and S3-P2 is not used to judge it.

## 4. Conclusion

With M1 (exposure CONFIRMED), M2 (fix, M2-PASS, 249 MB → 1.29 MB) and
M3 (PASS), Stage41 needs only M4: the `KNOWN_LIMITATIONS` update and the
closure.

## 5. Owner decisions

- Merge PR #117 (M2 fix, M2 record and this M3 record).
- Approve M4 (docs only).
