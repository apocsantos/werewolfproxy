# Stage33 QUIC and webpki dependency remediation

Owner decision, 2026-10-29 (Stage33 step 4, option a): the candidate's
advisory gate failed, so remediate before the `1.0.0-rc.2` candidate
audit, as `docs/STAGE33_RUSTLS_0_23_46_REMEDIATION.md` did. Dependency
change only; no Werewolf code changes. A tokio update in the first
attempt was withdrawn after a Stage32 regression (owner decision b,
[below](#the-tokio-update-withdrawn)).

## The advisories

`cargo audit` 0.22.2 on the step 4 release inputs (RustSec database
commit `ee1a49a2`, 2026-10-29 15:05 +01:00, 1386 advisories; 224 lockfile
dependencies) reported seven vulnerabilities and one warning.

| Advisory | Package | Fixed in | Shipped | Exposure in Werewolf before the change (code reading) | Outcome |
|---|---|---|---|---|---|
| RUSTSEC-2026-0341: stream reassembly memory guard bypass (CVSS A:H) | quinn-proto 0.11.15 | ≥ 0.11.17 | yes | likely: reachable by any QUIC peer before Werewolf authentication | fixed |
| RUSTSEC-2026-0342: zero-length DATAGRAM frames bypass the receive buffer limit | quinn-proto 0.11.15 | ≥ 0.11.17 | yes | likely: Werewolf does not disable QUIC datagrams | fixed |
| RUSTSEC-2026-0339: unbounded pending connection-ID retirements | quinn-proto 0.11.15 | ≥ 0.11.17 | yes | likely | fixed |
| RUSTSEC-2026-0340: ACK_FREQUENCY minimum ACK delay panic (CVSS A:H) | quinn-proto 0.11.15 | ≥ 0.11.18 | yes | probably not: ACK_FREQUENCY is not enabled | fixed |
| RUSTSEC-2026-0338: zero-length STREAM frame panic in unordered reads | quinn-proto 0.11.15 | ≥ 0.11.18 | yes | probably not: Werewolf reads streams in order | fixed |
| RUSTSEC-2026-0382: distrusted intermediates accepted | rustls-webpki 0.103.15 | ≥ 0.103.16 | yes | probably not: exact-SPKI pinning, no CA chain building | fixed |
| RUSTSEC-2026-0383: distrusted emSign intermediates | webpki-root-certs 1.0.7 | ≥ 1.0.10 | no | not exposed: not in the release closure | fixed |
| RUSTSEC-2026-0386 (warning, unsound): `AsyncFd` I/O safety | tokio 1.52.3 | ≥ 1.53.3 | yes | not reachable: neither Werewolf nor any shipped dependency other than tokio uses `AsyncFd` | **accepted** (owner decision b) |

The exposure column is a reading of the code, not a proof; the
remediation does not depend on it. The `AsyncFd` check is a search of
the Werewolf crates and of the source of every crate in the Linux
release closure of `werewolfd`. The warning is left visible (no
`cargo audit` ignore): an unsound warning does not fail the gate, and an
ignore would hide it from later audits.

## The change

| Dependency | Before | After |
|---|---|---|
| `quinn` (workspace lock; `quinn = "0.11"` unchanged) | 0.11.9 | 0.11.12 |
| `quinn-proto` | 0.11.15 | 0.11.18 |
| `rustls-webpki` in `crates/werewolfd/Cargo.toml` | `=0.103.15` | `=0.103.16` |
| `webpki-root-certs` (lockfile only) | 1.0.7 | 1.0.10 |
| `tokio` | 1.52.3 | unchanged (1.53.3 withdrawn) |
| `rustls` | `=0.23.46` | unchanged |

Each move is a narrow `cargo update -p <crate> --precise <version>`.
quinn-proto 0.11.18 is the first version that fixes all five quinn-proto
advisories; quinn 0.11.12 is the quinn release that requires
`quinn-proto ^0.11.18`, so the pair is the one upstream released
together rather than quinn 0.11.9 over a newer proto.

quinn-proto 0.11.18 itself requires `rand` 0.10, `fastbloom` 0.17 and
`rustls-platform-verifier` 0.7, so the lockfile moves further than the
named crates (224 → 218 packages):

- updated: fastbloom 0.14.1 → 0.17.0, jni 0.21.1 → 0.22.4, rand 0.9.4 →
  0.10.3, rand_core 0.9.5 → 0.10.1, rustls-platform-verifier 0.6.2 →
  0.7.1, rustls-platform-verifier-android 0.1.1 → 0.2.0;
- added: chacha20 0.10.2, foldhash 0.2.0, getrandom 0.4.3, jni-macros
  0.22.4, portable-atomic 1.15.0, r-efi 6.0.0, rand_pcg 0.10.2,
  simd_cesu8 1.2.0, simdutf8 0.1.5;
- removed: cesu8, jni-sys 0.3.1, ppv-lite86, rand_chacha 0.9.0,
  windows-sys 0.45.0 and its windows-targets 0.42.2 family, zerocopy and
  zerocopy-derive 0.8.48.

Of these, the Linux release closure of `werewolfd` gains quinn 0.11.12,
quinn-proto 0.11.18, rand 0.10.3 / rand_core 0.10.1 / rand_pcg 0.10.2,
chacha20 0.10.2 (rand's generator), getrandom 0.4.3, fastbloom 0.17.0,
foldhash 0.2.0, portable-atomic 1.15.0 and rustls-platform-verifier
0.7.1, all through quinn-proto. Werewolf's own key generation and
record encryption do not use any of them (`rand_core` 0.6,
`chacha20poly1305` 0.10.1 and its chacha20 0.9.1 are unchanged); the
platform verifier is not used, since Werewolf installs its own exact-SPKI
verifier. The jni, Android, Windows and UEFI entries are not built for
the Linux target.

rustls 0.23.46 is still the only rustls. The new `Cargo.lock` SHA-256 is
`a191a20e0e943f726884f5965b93814df4c59145d170de66e3b554ab0b36d27b`.

The Stage12 TLS and Quinn proof harnesses move to the same pins (`quinn
=0.11.12`, `rustls-webpki =0.103.16`, quinn-proto 0.11.18 in the harness
lockfile; tokio `=1.52.3` unchanged) and their report labels.

`cargo audit` after the change, same database: **0 vulnerabilities, 1
warning** (RUSTSEC-2026-0386, accepted above), 218 dependencies.

Previous step 4 archive hashes are obsolete; the release inputs are
rebuilt on this branch's final commit.

## The tokio update, withdrawn

The first commit (`c71bf24`) also moved tokio 1.52.3 → 1.53.3 for
RUSTSEC-2026-0386. Its Stage32 QUIC subset (results
`20261029T195020Z-network` and `20261029T200253Z-network`, build
`59f6dd98…2df`) stopped at **S3-P2**: on seed 2010 the routed TCP hop
reached the 120 s bound with 1,031,016 of 1,048,576 bytes echoed (last
echo at 57.6 s), no reset, no corruption, daemons alive. The certified
S3-P2 rerun passed all three runs (TCP hop 23.6–43.3 s).

**Diagnosis** (diagnostic, not certification evidence; evidence in
`target/stage33-diag/`): seed 2010 reproduced exactly as the cell runs
it (QUIC-hop leg, then TCP-hop leg, under the same netem qdiscs), with
tcpdump on A's and Y's links and `ss -tin` snapshots every second,
alternating builds:

| Build | `werewolfd` | Runs | TCP hop not complete within 120 s |
|---|---|---|---|
| `main` `efb537d` (quinn 0.11.9, tokio 1.52.3) | `38805ea7…675` | 14 | 0 (23.2–74.2 s) |
| this remediation with tokio 1.52.3 | `fe66ba76…5c5` | 8 | 0 (29.6–99.5 s) |
| this remediation with tokio 1.53.3 (`c71bf24`) | `59f6dd98…2df` | 18 (with the campaign's) | 6 (five at the bound still progressing, one reset at 119.7 s) |

The stall is in kernel TCP. When B has echoed everything, Y's daemon
has relayed every byte and closed its side; Y's kernel still holds
about 70–160 KB of echo toward A, which drains over a path that
crosses four 10% loss qdiscs. In a failing run, A's ACK of the last
data was lost before Y, and Y's retransmission timer had backed off to
17 s and then 32 s. The same tail appears on every build (up to 47 s on
`main`), longer and more often with tokio 1.53.3. The tokio 1.53.0–1.53.3
changelogs list no TCP or I/O write change (timer and waker-ordering
changes only); the cause of the correlation is not identified.

**Owner decision (b):** hold tokio at 1.52.3 and accept
RUSTSEC-2026-0386 as above; keep the quinn and webpki updates.

## S3-P2 on the final commit

The subset on `e2ce899` (results `20261029T231419Z-network`, build
`fe66ba76…5c5`) stopped at S3-P2 again: seed 2010 PASS (TCP hop
32.2 s); seed 2020 PASS as a clean early end (TCP hop reset at
118.6 s); seed 2030 FAIL, the TCP hop at the 120 s bound with 961,400
of 1,048,576 bytes echoed in order, last echo at 110.4 s.

**Diagnosis**, the same method on seeds 2020 and 2030, `main` and the
final build alternating, eight runs per seed and build:

| Seed | `main` `efb537d`: TCP hop at the bound | final `e2ce899`: TCP hop at the bound |
|---|---|---|
| 2010 (above) | 0 of 14 | 0 of 8 |
| 2020 | 2 of 8 | 1 of 8 |
| 2030 | 0 of 8 | 1 of 8, and the campaign run |
| all | 2 of 30 | 3 of 27 |

Every run at the bound, on either build, has the tail described above:
Y's daemon has relayed everything and closed, and Y's kernel spends
about 100 s retransmitting the last 60–130 KB toward A. Every echoed
byte matched, in order, and no run was stuck (`main` seed 2020 was
still echoing at 119.99 s). Both builds also had setup resets at
6–12 s (clean early ends). The 1 MiB S3-P2 workload is marginal on
every build: the certified 3 of 3 does not show a failure rate of
about 7% per run. This is not a regression of this remediation.

**Owner decision (a):** S3-P2 is **EXCLUDED** on this commit, a
marginal workload on every build, evidence retained; the subset
resumes from S3-P5. The Stage32 record notes that S3-P2's certified
1 MiB payload is marginal; recalibration is left for later.

## Validation

On the final commit (tokio 1.52.3):

- `cargo fmt --check`, `cargo clippy --locked --workspace --all-targets
  -- -D warnings`: pass.
- `cargo test --locked --offline --workspace`: 412 passed, 0 failed
  (daemon 297, core 94, and every other target), including the
  RUSTSEC-2026-0285 regression and the TLS identity tests.
- Stage12 proof harnesses: the isolated TLS 1.3 authentication matrix
  (`stage12_tls_auth`, debug and release) `PASS`, 8 of 8 cases, webpki
  0.103.16 (it has no tokio; run on `c71bf24`); the Quinn authentication
  proof (`stage12_quinn_auth`) `COMPLETE`, UDP environment `PASS`, 3 of 3
  cases, quinn 0.11.12 / quinn-proto 0.11.18, tokio 1.52.3.
- Stage lab gates (disposable daemons, commit `e2ce899`): all ten pass.
  `service_gate` (10), `discovery_gate` (12), `service_discovery_gate`
  (7), `routed_fang_gate` (12), `provider_gate` (7), `governance_gate`
  (6), `activation_gate` (4), `sync_gate` (5), `death_gate` (6),
  `auto_sync_gate` (9).
- Stage32 QUIC subset (quinn is the transport Stage32 certified):
  certification build of `e2ce899`, `werewolfd` `fe66ba76…5c5`, cells
  from the certified matrix that carry QUIC, P3 cells left out (already
  EXCLUDED). Results `20261029T231419Z-network` and, resumed after the
  S3-P2 decision, `20261031T222111Z-network`.

  | Cell | Result |
  |---|---|
  | S2-P0, S2-P1, S2-P2, S2-P5 | PASS, 3 of 3 each |
  | S3-P0 | PASS, 3 of 3 |
  | S3-P2 | EXCLUDED (owner decision, [above](#s3-p2-on-the-final-commit)) |
  | S3-P5 | EXCLUDED under F3 by the harness rule, as certified: 5010 and 5020 PASS (QUIC hop 107.2 s and 118.5 s, TCP hop 74.0 s and 88.8 s); 5030 QUIC hop at the bound still progressing (1,876,224 of 2,097,152 B, last echo at 119.9 s), TCP hop complete in 82.4 s |
  | S4-P0, S4-P2 | PASS, 3 of 3 each |
  | S7-P2 | PASS, 3 of 3 |

  Not rerun: S2-P4 (EXCLUDED under F3), S2-P6 to S2-P9, S3-P1, S3-P4,
  S3-P6 to S3-P9 and the other S4 profiles, and the S1, S5 and S6
  surfaces, which carry no QUIC change beyond these.

**S2-P3, observation** (owner decision: recorded under the existing P3
decision, "P3 transfer availability is a documented limit; S1-P3, S2-P3
and S3-P3 are EXCLUDED"). S2-P3 was included in the first subset run
by mistake, since the cell is already excluded. On `c71bf24` seeds 3010
and 3030 ended early (QUIC reset at 78.7 s and 53.2 s; 4.8 MB and 1.7 MB
echoed in order), and seed 3020 crawled to the 120 s bound (11.5 MB of
32 MiB echoed in order, last echo at 103.1 s), daemons alive, no leaks,
no corruption. With quinn 0.11.9 the certified runs all ended early
(51.0 s, 56.1 s, 33.8 s). Both are within "sessions end early or
crawl". P3 cells are not part of the subset.
