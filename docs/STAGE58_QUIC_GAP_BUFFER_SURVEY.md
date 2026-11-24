# Stage58 — routed QUIC gap-buffer follow-up survey

**Decision:** S3-P2 remains **EXCLUDED**. No production transport or
dependency change is proposed by this survey. The evidence identifies a
credible upstream lead for the loss-related assembler behavior, but does not
establish that it caused Werewolf's setup resets or slow transfers.

## Evidence

Stage34 records a reproducible regression on seed 2030 for routed QUIC with
10% loss on both impaired links. The regression begins with
`quinn-proto` 0.11.18 and persists on 0.11.19. Its leading hypothesis was
send-buffer accounting, but a 4 MiB diagnostic send window removed slow
transfers without removing setup resets. That observation leaves the cause
unproven; the larger window was not approved for production.

Upstream Quinn PR [#2976](https://github.com/quinn-rs/quinn/pull/2976),
opened November 2, 2026, proposes scaling the stream assembler's permitted
gap count with buffered data (minimum 1,024, one additional gap per KiB,
maximum 8,192). Its author says it addresses assembler rejection under
sustained packet loss. This is a symptom-level match to the Stage34 loss
scenario, not evidence that it fixes Werewolf's transfer crawl or handshake
resets. As checked November 24, the PR is still open. Quinn's releases page
still identifies `quinn-proto` 0.11.19 as latest.

## Bounded next step

1. Wait for an upstream merged change and a published `quinn-proto` release
   later than 0.11.19. Confirm from release notes and source that the candidate
   contains the relevant assembler change, and verify it retains the security
   fixes required by Stage50. An open PR or a git dependency is not a candidate.
2. Once available, prepare an isolated dependency trial that changes no
   application transport settings. Review the complete lockfile diff and
   security advisories before running any S3-P2 workload.
3. Reproduce the exact Stage34 seed-2030 cell sequence with its captures and
   at least six rounds, comparing against the recorded 0.11.17 behavior.
   Record setup resets, transfer progress, completion time, and assembler or
   transport diagnostics where available. Do not infer causality from a
   throughput-only improvement.
4. If the regression check passes, separately obtain authorization for the
   Stage34 certification campaign. The campaign's 30-run rule and kernel-tail
   accounting remain unchanged. S3-P2 stays excluded until that campaign
   passes and is recorded.

If upstream closes the PR without merging it, or releases a change that does
not contain the proposed gap-buffer behavior, update this survey with the
reason and continue treating the Stage34 send-buffer explanation as unproven.
No upstream issue or report is filed by this work.

## Boundaries

- No production dependency update, vendoring, transport-window change, or
  release recertification.
- No S3-P2 run is authorized or performed by this survey.
- S3-P2 remains excluded; the Stage34 retest and certification requirements
  remain authoritative.

## Sources

- [Stage34 S3-P2 record](STAGE34_S3_P2_RECORD.md)
- [Stage50 dependency security survey](STAGE50_DEPENDENCY_SECURITY_SURVEY.md)
- [Quinn PR #2976](https://github.com/quinn-rs/quinn/pull/2976)
- [Quinn releases](https://github.com/quinn-rs/quinn/releases)
