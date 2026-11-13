# Stage49 M3 — regression

**Functional regression: PASS. M3 gate: FAIL; closure blocked by the
dependency audit.** Fresh validation on
`0f5afa795591566ef570f226ec0b26a22cf9a623` covers the complete Stage49 contract: the refusal
matrix three times, Stage46/48 matrices, descriptor and quota floods,
Stage32 S2/S3/S4/S7 P0 three times each, and 12 of 13 local gates. The
audit gate reports three dependency advisories; it is not ignored. The
Rust-suite log reports 460 passed, including the new probe-port guard
test. The recovered Cargo exit-status limitation is stated in §5.

## 1. Tested changes and scope

- M2 (`267385f`, merged in #157) added the connected UDP refusal probe.
- Delivery fix `de0a8e3` prevents a probe using its own destination port:
  drop that socket and retry its bind once; another collision disables
  the probe through the existing local-error fallback. At most one probe
  socket is held and at most one one-byte datagram is sent.
- Timeout fixtures retain silent bound UDP sockets (`d280f5b`). The
  closed-port tests reserve available ports outside Linux's configured
  ephemeral range and hold them until endpoint creation (`0f5afa7`),
  excluding concurrent automatic allocation. This test fixture reads
  `/proc/sys/net/ipv4/ip_local_port_range`; it changes no host setting.
- Existing timing and exact failure-count assertions remain; the
  multi-address silent test also checks the lower bound of the existing
  read window. No dependency, wire-message, authentication, Target
  Policy, schema, timeout or probe socket/datagram ceiling change.
- Fresh lab daemon SHA-256: `fdc10bc1d6728fb00da9b9ab7c3d5edaf2a59667d30a470be48fc897de5cc0e4`.
  Evidence: `target/stage49-postfix/`, ending 2026-11-13T13:40:54.565697+00:00.

## 2. Stage49 matrix, three rounds

`tools/stage49/refused.py`: 31 scenarios per round, 3 clients per
connection scenario. Ranges across all three rounds:

| QUIC path | `refused` | `blackhole` | `fast` |
|---|---|---|---|
| direct | reset, 0.00 s | reset, 5.00 s | echo, 0.02–0.03 s |
| one unavailable provider, then C | echo, 0.02–0.03 s | echo, 5.05–5.06 s | echo, 0.02–0.03 s |
| two unavailable providers, then D | echo, 0.02–0.03 s | echo, 10.05–10.06 s | echo, 0.02–0.03 s |
| routed | reset, 0.00 s | reset, 5.00 s | echo, 0.04 s |
| discovery | `unreachable`, 0.01 s | `unreachable`, 5.01 s | `ok`, 0.03 s |
| governance sync | `unreachable`, 0.01 s | `unreachable`, 5.01 s | `ok`, 0.04 s |

Every refused QUIC path, including discovery and sync, meets the
under-0.1 s criterion. M1 cost 5 s per refused provider and 0.30–0.36 s
for `fast`; see [M1](STAGE49_M1_QUIC_REFUSED_REPRODUCTION.md). TCP and
silent-peer controls retain their expected outcomes and timeout windows.
All daemons survived.

Each round also sent 10,000 one-byte datagrams to a live QUIC receiver:
zero log lines and 6/6 clients echoed before/after in every round.
Evidence: `refused-r{1,2,3}.json` and matching logs.

## 3. Stage46/48 matrices and resource pressure

- **Stage46:** all 40 scenarios, 5 clients each, reached the expected
  result. Refused first addresses advance without the stagger; silent
  first addresses retain it; dead peers retain their existing windows.
  Evidence: `dial.json` and `dial.log`.
- **Stage48:** all 18 scenarios, 3 clients each, reached the expected
  result. One silent encrypted-TCP provider then C:
  5.03–5.06 s;
  two then D: 10.03–10.04 s.
  Direct silent TCP retains its F1 retry. Evidence: `retry.json` and log.
- **QUIC fd limit 64, three rounds:** flooded descriptors
  64 / 64 / 64; saturated CPU
  1.4% / 1.4% / 1.6%. Every round
  recovered to 18 descriptors, a listening/listed Fang and legitimate
  echo, with its daemon alive. Evidence: `fd64-r{1,2,3}.json`.
- **QUIC fd limit 256 and quota:** both recovered to 18 descriptors and
  legitimate echo; quota descriptors stayed at 18 after 30 s. A full
  quota may refuse a legitimate client during the flood, as recorded in
  Stage42. Evidence: `fd256.json`. No sustained spin or leak appeared.

## 4. Stage32 P0 regression

`chaos.py network --cells S2:P0,S3:P0,S4:P0,S7:P0`: **12/12 runs PASS**,
3 per cell. Evidence: `chaos.log` and
`/home/sleepdeprivedloon/Projects/werewolfproxy/target/stage32-chaos/results/20261113T132439Z-network`. Campaign provenance records the tested commit and
the separate certification/lab-feature binaries. S3-P2 and S2-P4 were
not retested or recertified; their exclusions remain unchanged.

## 5. Rust and local gates; restart recovery

Formatting, Clippy and the locked offline release build exited 0.
The retained full workspace/all-features Rust output reports **460
passed, zero failed**, followed by complete core doc-tests.

The execution server restarted while Cargo was running. Cargo survived
as an orphan and finished, but its exit status was unavailable to the
replacement driver. Recovery verified the complete results and recorded
`recovered-output` in `summary.tsv`; it did not invent an exit code.
A separate workspace/all-features doc-test invocation exited 0.
`rust-recovery-note.txt` preserves this limitation.

Full disposable local CI: **12/13 PASS; audit FAIL**, exit 1. The other
twelve checks exited 0. The refreshed database at
`4ef2df8e806a65e2f1ecc14f56f51249062b23d6` flags RUSTSEC-2026-0523 and -0524 in
quinn-proto 0.11.18 and RUSTSEC-2026-0538 in quinn-udp 0.5.14. The UDP
advisory explicitly excludes Linux, but remains a lockfile audit finding.
No dependency or advisory-ignore entry was changed. The committed
upstream records and audit log are retained under `advisories/` and
`audit-blocker.log`. This replaces the earlier all-clear audit result
with a fresh finding; the advisories were not introduced by this PR.
Evidence: `target/ci-local/20261113T133214Z/summary.tsv`, individual logs,
`ci-local.log` and `outcome.json`.

GitHub Rust CI and CodeQL jobs for PR #158 could not start because the
account was locked by a billing issue. No hosted test or scan pass is
claimed. Stage40's local-CI decision remains in effect; hosted workflows
and billing settings were not changed.

## 6. Earlier evidence and delivery failures

Original M2-source M3 scenarios completed on 2026-11-12, 08:14–08:51 UTC,
before a host crash. Their JSON, logs and daemon hash `c484a543…` survive
in `target/stage49-m3/`. Recovery verified them and completed local CI
13/13 at `target/ci-local/20261112T091031Z`. Interrupted empty logs at
`20261112T085104Z` supply no verdict; sandbox infrastructure failures at
`20261112T090954Z` are not product verdicts.

Fresh delivery validation then exposed invalid closed-port fixtures:

| Attempt | Observed failure | Evidence |
|---|---|---|
| `9438165` | old no-answer test unwrapped a local connection that had already reset; daemon suite 342 passed, 1 failed | `target/stage49-delivery/` |
| `d280f5b` | new refused-address race took 5.002 s; daemon suite 342 passed, 1 failed | `target/stage49-delivery-fixtures/` |
| `de0a8e3` | race again took 5 s; closed-port probe timed out; daemon suite 342 passed, 2 failed | `target/stage49-postfix-first/` |

Those failures were retained and were not retried away or counted as
passes. Their logs did not identify the owner of a reused port, so
attribution of the original failures to a particular socket remains
unproven. A separate 1,000-probe diagnostic received 1,000 refusals.
The independent self-probe case was reproduced deterministically with
a wildcard UDP socket connected to its own loopback port:
`self-probe-reproduction.json`. The fixture corrections and bounded
production guard are validated by the fresh contract above.

## 7. Closure boundaries

The functional criteria hold on the corrected source, but the required
audit gate fails. Stage49 remains unmerged and unclosed pending an
approved dependency remediation or explicit owner disposition. M4 updates
[known limitations](../KNOWN_LIMITATIONS.md) and the
[closure](STAGE49_CLOSURE.md). Filtered/absent ICMP and silent peers keep
the 5 s window; forged ICMP can end a dial early. No WAN check, dependency
promotion, new release or historical verdict change is included. The
next-priority proposal is the [Stage50 security survey](STAGE50_DEPENDENCY_SECURITY_SURVEY.md).
