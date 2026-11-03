# Stage39 M3 — regression of the target-connection liveness fix

## 1. Result

**M3: PASS.** The M2 fix (`3f1477a`) introduced no regression in the
authorised regression scope:

- the ten lab gates and the Stage12 proofs;
- the Stage22R D07 target-failure cases, run in the sandbox, direct and
  through a relay, on both transports;
- the Stage32 network cells whose sessions use a target connection.

Every cell has the same verdict as in its historical record:
- PASS where it was PASS;
- EXCLUDED where it was EXCLUDED under F3;
- S2-P4: the harness says FAIL and the owner's historical classification
  is EXCLUDED (F3). The behaviour matches the historical record, and it is
  not reclassified here.

No production code, harness rule, profile, bound or historical verdict
changed. S3-P2 stays EXCLUDED and was not a criterion.

## 2. Build and baseline

| | |
|---|---|
| `main` | `aff205aa3ba96e0fef8ccb624bcfde263f93142c` (#111 merged); production source identical to the M2 candidate `3f1477a` |
| `werewolfd` | `351db99f164dce10a97a029dea2ea6b39cbb12887dd5c06719c2ce893f7122fd` (release, `--locked`); the Stage32 harness built the same hash itself from `aff205a` |
| Unchanged | tokio 1.52.3, quinn-proto 0.11.18, `Cargo.lock`; Stage38 CLOSED; `v1.0.0-rc.2` frozen |
| Host | Linux 6.12.113+deb13-amd64, rustc 1.96.0; no sysctl changed |

## 3. Lab gates and Stage12 proofs

| Gate or proof | Result |
|---|---|
| service, discovery, provider, governance, activation, sync, death, auto_sync gates | PASS |
| service_discovery gate | PASS on rerun (7 of 7); see below |
| routed_fang gate | PASS on rerun (12 of 12), including 64 MiB full duplex and target-first half-close on both hops, Silver on the relay, and a relay address change |
| Stage12 TLS 1.3 matrix, debug and release | PASS |
| Stage12 Quinn proof, release | COMPLETE, UDP environment PASS |

**First runs of two gates were invalid:**

- **What happened:** the first run of `service_discovery_gate` and of
  `routed_fang_gate` failed only their "source preservation" check, with
  every functional test passing (7 of 7 and 12 of 12). I had created the
  M3 D07 tooling files in the repository tree while those two gates were
  running, so the gates' own before-and-after check saw the tree change.
- **What it means:** this was an operator error, not a product
  behaviour.
- **What was done:** both gates were rerun with the tree untouched, and
  passed in full. The failed first runs are kept in the evidence.

## 4. D07 target-failure equivalents (sandbox)

D07 (Stage22R, real WAN) never ran in the sandbox or through a relay.
`tools/stage39/d07_sandbox.py` reproduces it:

- **Same target behaviours:** `echo`, `rst`, `kill`, `fin`, `absent`.
- **Same sizes:** a 64 MiB upload, with the target acting after 8 MiB.
- **Same series:** echo ×3, rst ×5, kill ×5, fin ×5, absent ×3.
- **Same pass rules and `kill` exclusion,** fixed before the runs.
- **Added:** after every run, every receiver's (and the relay's) lease
  must return to 0 within 10 s. After a 35 s settle the Stage32 leak
  bounds apply.

| Path | Runs | PASS | EXCLUDED | FAIL | Lease cleared after each run | Leak after settle |
|---|---|---|---|---|---|---|
| direct, encrypted TCP | 21 | 21 | 0 | 0 | at most 0.2 s | none (fds +0) |
| direct, QUIC | 21 | 21 | 0 | 0 | at most 0.2 s | none |
| routed, encrypted-TCP hop | 21 | 21 | 0 | 0 | 0.0 s | none |
| routed, QUIC hop | 21 | 21 | 0 | 0 | 0.0 s | none |

The observed endings match D07's record:

| Mode | Endings observed |
|---|---|
| echo | exact 64 MiB |
| rst, kill | a reset, with exact prefixes of 5.6–8.4 MiB; no session looked complete |
| fin | EOF after exactly 8,388,608 bytes. Direct QUIC's upload completed, because its stream directions are independent; the encrypted-TCP and routed paths ended the send direction with EPIPE, the session-end design |
| absent | an immediate reset, 0 bytes |

No `kill` run showed the ambiguous FIN pattern this time.

## 5. Stage32 network subset

The cells are the ones whose sessions use a target connection, three
seeds each under the unchanged Stage32 rules (`chaos.py network --cells`).
Results are in `target/stage32-chaos/results/` runs
`20261103T191212Z`, `20261103T194643Z` and `20261103T195252Z`.

| Cell | M3 verdict | Historical status | Note |
|---|---|---|---|
| S1-P0, P1, P2, P4, P5 | PASS 3/3 each | PASS | |
| S2-P0, P1, P2, P5 | PASS 3/3 each | PASS | |
| **S2-P4** | harness FAIL 3/3 | **EXCLUDED (F3)** by owner decision 1a (Stage32 spec) | see below |
| S3-P0, P1 | PASS 3/3 each | PASS | |
| S3-P4, P5 | EXCLUDED 3/3 each (F3, harness-enforced) | EXCLUDED (F3) | the TCP-hop leg completed with an exact match in all 6 runs |
| S3-P2 | PASS 3/3 (run separately) | **EXCLUDED** (Stage34) | not a criterion |
| S4-P0, P2 (establishment) | PASS 3/3 each | PASS | 50 opens per run per transport; each open makes a target connection |
| S6-P0, P2 (fences) | PASS 3/3 each | PASS (certified after the S6 harness fix) | Silver and revoke during transfers |

**S2-P4.** All three runs reached the 120 s bound still progressing:
- the last echo came 119.9–120.0 s in;
- 18.2–19.6 MB of 32 MiB was echoed, in order;
- no leak; the daemons stayed alive.

The Stage32 record is the same: harness FAIL, all three at the bound,
19.4–19.7 MB echoed. The owner classified it EXCLUDED as the documented
limit of QUIC throughput under reordering. The harness's F3 rule covers
only routed hops, so for direct S2-P4 it still reports FAIL. Including
S2-P4 in the M3 cell list was my mistake, because its status was already
known. The harness stopped there under its own rule, and the remaining
cells were run as a second invocation. Nothing is reclassified.

## 6. Conclusion

The fix changes no behaviour outside the defect it corrects:

- authentication and authorization still work;
- the target policy grant still works;
- establishment, half-close, Silver and revoke fences still work;
- all target-failure endings behave as before;
- throughput under impairment is unchanged.

The improvement itself is in [M2](STAGE39_M2_TARGET_LIVENESS_FIX.md):
relay-to-destination path loss now ends in 56–81 s with a reset.
`KNOWN_LIMITATIONS.md` records:
- the defect, as it exists in 1.0.0-rc.2;
- the fix on `main`;
- that it has been exercised in the sandbox only.

## 7. Limitations

- **Sandbox only:** no real-WAN run (S39-D5, separately authorised).
- **Not rerun:** the Stage32 cells outside the regression scope (P3,
  P6–P9, S5, S7) and the time campaign. They do not depend on the target
  connection's socket options.
- **Not in the release:** the fix is on `main`, not in the rc.2 archive.
  No release or rc.3 is certified by this record.

## 8. Evidence

Kept outside Git, in `target/stage39-m3/`:
- `MANIFEST.txt` (SHA-256 of every log and record);
- `gates/`, including the two invalid first runs and their reruns;
- `d07/*.json`;
- the Stage32 logs and cell lists.

The Stage32 per-cell records are in
`target/stage32-chaos/results/20261103T*`. The D07 tooling is in
`tools/stage39/` (`d07_sandbox.py`, `d07_target.py`, `d07_client.py`).

## 9. Owner decisions

- Merge this record, and the `KNOWN_LIMITATIONS` update.
- **Optionally**, authorise a real-WAN relay-to-destination check
  (S39-D5).
- Stage39 can then close, or proceed to the next stage from the Stage39
  survey.

Not authorised: WAN testing, tokio promotion, reopening Stage38, rc.3,
and release certification.
