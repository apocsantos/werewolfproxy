# Stage50 — isolated dependency trial, step 2

**Delivery note (2026-11-16):** after reviewing this validation, the owner
approved publication and merged [PR #159](https://github.com/apocsantos/werewolfproxy/pull/159)
as `c1e7845`. The patched graph is now on `main`; validation inputs match
the tested candidate. See the [closure](STAGE50_CLOSURE.md). The step-2
scope and isolated-candidate statements below describe the trial before
that later approval and merge.

**Result: functional validation PASS on the isolated candidate; one
timing outlier is retained for review.** The workspace and Quinn
proof audits pass, all 460 Rust tests pass with an actual exit 0, both
Stage12 proofs pass in debug and release, the transport/resource/P0
controls pass, and full local CI is **13/13 PASS**. This is validation
evidence for review, not promotion of the candidate or a new release.

## 1. Candidate and scope

- Branch: `stage50/dependency-trial`; tested commit
  `7ed3a91fd594f4ea49b7e336d151d076c214e0fc`, based on owner-merged Stage49 #158 (`72cc0e0`).
- User selected step 2: audit-first candidate validation. The two
  precise updates and proof alignment are its necessary prerequisites.
  No main merge, publication or release replacement is part of this step.
- quinn-proto **0.11.18 → 0.11.19**; quinn-udp **0.5.14 → 0.5.16**.
  Quinn remains 0.11.12, Tokio 1.52.3 and Rustls 0.23.46. Neither root
  manifest nor the advisory-ignore list changed.
- The independent Quinn proof lockfile uses the same two versions;
  only its emitted version labels changed in test source. Assertions,
  authentication rules, application source and transport bounds did not.
- The old UDP dependency's Windows 0.60/0.53 subgraph was removed.
  Five workspace and two proof-harness unrelated Windows selections
  were preserved. Both complete graphs pass locked offline metadata
  resolution; no other package version was updated. Windows runtime
  compatibility was not tested.
- Candidate root lock SHA-256: `411ef3b496dfd8c21f97eb2285d55bd0d12ae604436acfdfa16044e80a0d91d0`.
  Disposable lab daemon SHA-256: `8f1bdb20d54f3babeee487427ccb9a29d4fc4c5d7d08d5b61288f732833995a9`.

## 2. Advisory and source checks

Fresh `cargo audit --deny warnings` exited 0 for the workspace, and the
Quinn proof lockfile audit exited 0 against the same database. Full local
CI refreshed the audit again and passed. Because that database changed,
the proof lockfile was re-audited against the final snapshot and exited
0 too (`proof-audit-refreshed.result.json`). Database after validation:
`02e2517f457a359fc6d3440ea4560c8ea188fee8`. The pre-existing accepted Tokio advisory remains the only ignore;
no new exception was added.

The three original findings are documented in the
[security survey](STAGE50_DEPENDENCY_SECURITY_SURVEY.md). Source review
matched the candidate files to checksum-verified registry archives:

- **RUSTSEC-2026-0523:** bidirectional MAX_STREAM_DATA handling validates
  the stream ID against the receive limit before implicitly opening it.
- **RUSTSEC-2026-0524:** Initial packet construction rejects a peer token
  that leaves insufficient space for required frames, rather than relying
  on the debug assertion or attempting an excessive allocation.
- **RUSTSEC-2026-0538:** the single-datagram receive loop rebuilds its
  header per iteration and processes receive success/errors before the
  truncation flag. The upstream record explicitly excludes Linux; this
  trial does not claim to have exercised an affected Apple/BSD path.

Evidence: `upstream-fix-review.json`, `audit.log`,
`audit-quinn-proof.log` and the final local-CI audit log. This step
reviewed upstream fix locations and ran the existing proofs; it did not
execute new exploit demonstrations against Werewolf.

## 3. Rust, proofs and integration

| Check | Result |
|---|---|
| Workspace and Quinn-proof formatting | PASS |
| Workspace/all-targets/all-features Clippy, warnings denied | PASS |
| Locked offline workspace/all-features Rust suite | **460 passed**, exit 0 |
| Locked offline workspace/all-features release build | PASS |
| Stage12 TLS authentication proof | **8/8**, debug and release |
| Stage12 Quinn authentication proof | **3/3**, debug and release; UDP environment PASS |
| Full local CI | **13/13 PASS**, every check exit 0 |

Full local-CI evidence: `target/ci-local/20261116T123910Z/summary.tsv`. Historical
Stage12 evidence JSON files were preserved; new proof output is under
`target/stage50-trial/proof-*.log`.

## 4. Transport and resource controls

Stage49 tool: **31/31 scenarios**, 3 clients per connection scenario.
Times below are per-client or per-query ranges:

| QUIC path | `refused` | `blackhole` | `fast` |
|---|---|---|---|
| direct | 0.00 s | 5.00 s | 0.02–0.03 s |
| one unavailable provider, then C | 0.02 s | 5.03–7.44 s | 0.02–0.03 s |
| two unavailable providers, then D | 0.02–0.03 s | 10.05–10.06 s | 0.02 s |
| routed | 0.00 s | 5.00 s | 0.04 s |
| discovery | 0.01 s | 5.01 s | 0.03 s |
| governance sync | 0.01 s | 5.01 s | 0.04 s |

The initial blackholed-provider failover range includes one **7.44 s**
client. Three fresh targeted repeats (nine clients) all echoed in
5.03–5.07 s. The original remains
in the table and evidence; it was not discarded or retried into a pass.
Its run spanned long pauses, but the cause of that specific delay is
unproven. These observations support the functional result, not a
comparative liveness or non-inferiority certification. Evidence:
`timing-followup.json` and `blackhole-repeat-{1,2,3}.*`.

Every refused path meets the original under-0.1 s criterion, including
discovery and sync. TCP and silent controls retain their expected
outcomes. All daemons survived. A live receiver discarded 10,000
one-byte datagrams with zero log lines and all six before/after clients
echoing.

- **Address controls:** 10/10 QUIC family-order and silent-first-address
  scenarios, 5 clients per connection scenario, expected outcomes.
- **Resource controls:** fd limit 256 and quota pass; all three fd-limit
  64 repeats pass. Saturated counts:
  64 / 64 / 64 descriptors; CPU
  1.6% / 1.6% / 1.4%. All recover
  to 18 descriptors and serve a legitimate connection after release.
- **Stage32 S2/S3/S4/S7 P0:** 12/12 runs PASS, 3 per cell. Campaign:
  `/home/sleepdeprivedloon/Projects/werewolfproxy/target/stage32-chaos/results/20261116T123134Z-network`.

## 5. Recovery, limits and disposition

The validation spanned long pauses between 2026-11-13 and 2026-11-16.
Checkpoints preserved completed checks rather than repeating them. Each
step's independent worker recorded its actual exit in `*.result.json`;
the driver validated scenario/proof outputs before writing
`verified.json`. Recorded durations use a monotonic clock and are not a
claim of continuous wall-clock performance during the pauses.

The host resource check on 2026-11-16 found approximately 6.1 GiB
available memory, 32 GB free disk, a 524,288 descriptor limit and a
31,110 process limit. The runtime resource tests used their explicit
per-daemon limits above. No host limit was changed.

S3-P2 and S2-P4 stay EXCLUDED. The known Stage34 loss limitation was
not recertified or claimed fixed; Stage38 stays CLOSED. Frozen rc.1/rc.2
tags and release artifacts were not modified, and no WAN or installed
node was used. The candidate remains isolated for the next review step;
these passing results do not by themselves change main's dependency
graph or close its historical audit finding.

Evidence root: `target/stage50-trial/`; validation completed
2026-11-16T12:45:17+00:00. `validation-meta.json`, `summary.tsv`, individual
worker results and `verified.json` identify the tested source and gates.
