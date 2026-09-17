# Stage 20: adversarial soak, chaos, and fault certification

## Scope and tested release

Stage 20 exercised the Linux x86_64 release-mode daemon and CLI on private
loopback fixtures. The certified starting commit was
`2514e5a6f393d8ba746e4b7699f9a8e64e8399c7`; Cargo.lock remained
`09b18cac90690855e4bee28cabc59ee713f14baf50f2efbb20f2910ed46047b4`.
The tested source/test commit is `48b9f9ad0a37b91645fe7ed939892a4b8fef60dd`.
The final documentation commit follows that tested commit and changes no
program, test, dependency, CLI, persistence, or wire behavior.

The release is `1.0.0-rc.1`, target
`x86_64-unknown-linux-gnu`, built by rustc/cargo 1.96.0 on Debian GNU/Linux
13.6 (trixie), x86_64, glibc 2.41, kernel
`6.12.107+deb13-amd64`. These results do not certify another OS, architecture,
libc baseline, container runtime, or distribution.

Each campaign uses two separate temporary Dens and runtime directories,
generated disposable Pelt identities, dynamic loopback ports, and local echo
and hold targets. Provisioning uses werewolfctl. No Internet host or live user
Den is involved. Captured outputs contain only counts, timing, hashes of
generated data, resource samples, and test status; payloads, packet captures,
private identity files, and traffic secrets are not retained.

## Defects found and corrected

The initial short-connection churn exposed a Stage11C authority lease retained
after ordinary forwarding EOF. The route limit could then reject a later
connection. Commit `c123763` releases TCP/QUIC forwarding ownership on EOF and
adds regressions for both transports. The QUIC regression was extended to
10,000 sequential routes through one Fang activation; it passed in 163.99
seconds. A separate test verifies TCP release. This was a product resource
lifetime bug, not an authentication or wire-format defect.

A later single-topology run reset a TCP route after 9,176 steady cycles. The
reproducible observable was `Connection reset by peer`; isolated 10,000-cycle
TCP, 10,000-cycle QUIC, and 20,000-cycle mixed diagnostic loops had passed, so
short tests did not reproduce the longer-lifetime failure. Review found a new
Quinn client endpoint was being created for each local route and completed
Fang child `AbortHandle`s were retained. Commit `fadea72` reuses one Quinn
endpoint per Fang activation and closes each completed Quinn connection;
`2d87af7` prunes handles for completed children when tracking new work. The
10,000-route QUIC regression and 4,096-completed-task handle test pass. The
separate contribution of each resource path to the reset was not independently
proven. The final two-hour campaigns below passed after both fixes.

The earlier initial four-topology concurrent trial is excluded: one topology
reset under simultaneous contention at 1,378 steady cycles, while the same
seed passed in an isolated 1,036.036-second run with 5,503 cycles. It is
retained as contention-sensitive diagnostic evidence, not counted as a pass
or silently dismissed.

## Extended campaigns

Two post-fix campaigns ran for at least 7,200 seconds each; their harness
reported 7,236.004 and 7,236.150 seconds. Together they provide 14,472.154
seconds (4 h 1 min 12.154 sec) of post-fix extended operation. Adding the
earlier corrected 375.791-second campaign and the isolated 1,036.036-second
campaign gives 15,883.981 seconds (4 h 24 min 43.981 sec) of recorded extended
campaign time. The two long campaigns use independent recorded seeds and a
single topology each; they do not claim an arbitrary network-flood guarantee.

| Measure | Seed 20260923 | Seed 20260925 |
|---|---:|---:|
| Result / elapsed | PASS / 7,236.004 s | PASS / 7,236.150 s |
| Steady authenticated route cycles | 83,280 | 81,563 |
| TCP / QUIC reconnects | 1,000 / 1,000 | 1,000 / 1,000 |
| Alternating TCP/QUIC cycles | 1,000 | 1,000 |
| Peer / both-peer restarts | 100 / 10 | 100 / 10 |
| SIGKILL recoveries (idle / establish / TCP / QUIC) | 30 / 22 / 19 / 29 | 25 / 26 / 28 / 21 |
| Silver toggles (TCP / QUIC) | 10 (5 / 5) | 10 (5 / 5) |
| Pack revocations (TCP / QUIC) | 1 / 1 | 1 / 1 |
| Target revocation / Fang churn cycles | 20 / 20 | 20 / 20 |
| Manifest mutations / mutation-crash restarts | 40 / 6 | 40 / 6 |
| Malformed TCP / QUIC datagrams | 64 / 64 | 64 / 64 |
| Pre-auth / authority saturation recovery | 3 / 1 | 3 / 1 |
| QUIC stream churn / local control operations | 32 / 1,000 | 32 / 1,000 |
| 50 MiB transfers (TCP / QUIC) | 10 / 10 | 10 / 10 |
| Long-lived transfer ticks (TCP / QUIC) | 35 / 35 | 35 / 35 |
| Verified application bytes | 2,793,564,524 | 2,777,736,328 |
| SHA-256 checks | 86,302 | 84,585 |
| Active / establishing authority at checkpoints | 0 / 0 | 0 / 0 |
| FD baseline / peak / post | 22 / 24 / 22 | 22 / 24 / 22 |
| RSS baseline / peak / post (KiB) | 8,064 / 26,660 / 26,660 | 8,096 / 29,816 / 29,816 |
| CPU ticks in final one-second idle sample | 0 | 0 |
| Den bytes before / after | 4,675 / 5,270 | 4,675 / 6,155 |
| Log bytes baseline / peak / final | not sampled | 1,608 / 287,047 / 287,047 |
| Malformed-traffic log growth | not sampled | 0 bytes |

Both runs also passed strict secure-transport fallback checks, compatibility
matrix checks, one controlled port collision/recovery, one network-loss cycle
per transport, concurrent administrative mutation, and the restart/crash
checks listed above. Seed 20260925 used the strengthened harness: it maintained
live routes while activating Silver and revoking Pack authority, then required
post-linearization application writes to receive no echo. It also bounds
malformed-input log growth and uses constant-memory digest accounting.

The preceding corrected seed 20260916 run passed 1,000 reconnects per
transport, 1,000 mixed cycles, 100 peer restarts, 10 both-peer restarts, 100
SIGKILL recoveries, 10 Silver toggles, 20 target/Fang churn cycles, 40
generation mutations, 6 mutation-crash restarts, 10 50 MiB transfers per
transport, and 2,109,832,936 verified bytes in 375.791 seconds. It observed
21/23/20 FDs, 8,024/42,240/32,072 KiB RSS, zero final idle CPU ticks, and Den
growth from 4,675 to 5,978 bytes. This shorter campaign preceded the later
endpoint/child-handle fixes and is not substituted for the two final long
runs.

## Final release CI and final gates

The final packaged release binaries passed Stage20 CI with seed 20260926 in
76.152 seconds: 20 TCP, 20 QUIC, and 20 alternating cycles; three peer and one
both-peer restarts; five SIGKILL recoveries; active Silver and Pack fences on
both transports; three target/Fang churn cycles; six manifest mutations and
one crash recovery; 64 malformed TCP and 64 malformed QUIC datagrams; pre-auth
and authority saturation; 32 QUIC stream churn cycles; 100 control requests;
and one 50 MiB transfer per transport. The campaign verified 209,843,972
application bytes and 64 payload hashes. Malformed-traffic log growth was
zero; FDs were 22/23/22 baseline/peak/post, RSS was 8,064/15,576/15,576 KiB,
and final idle CPU ticks were zero.

Final source gates passed:

- `cargo fmt --check`;
- `cargo clippy --workspace --all-targets` (exit 0; existing lint warnings
  remain in unrelated core/daemon code);
- `cargo test --locked --offline --workspace`: 187 passed, 0 failed (22 core
  unit, 12 core integration, 153 werewolfd);
- Python integration unit suite: 64 passed;
- `python3 -B tests/integration/run.py`: acceptance 12/12;
- Stage17 lifecycle harness: PASS against packaged release binaries;
- Stage18 CLI provisioning harness: PASS against packaged release binaries;
- Stage19 archive/install harness and Stage18-to-candidate upgrade/downgrade
  harness: PASS;
- Stage20 CI harness: PASS against packaged release binaries;
- `git diff --check`: PASS.

The installed-artifact checks include fresh two-node CLI-only provisioning,
state-preserving software upgrade/downgrade, 50 MiB TCP and QUIC data
integrity, and uninstall preserving the Den. Stage17's actual systemd and
unprivileged transient-service tests remain PASS from its certification; this
Stage20 run did not repeat a long systemd restart-CHAOS campaign.

## Release artifacts and reproducibility

The exact artifact metadata is:

```text
release_version=1.0.0-rc.1
git_commit=48b9f9ad0a37b91645fe7ed939892a4b8fef60dd
source_date_epoch=1778563200
rustc=rustc 1.96.0 (ac68faa20 2026-05-25)
target=x86_64-unknown-linux-gnu
cargo_lock_sha256=09b18cac90690855e4bee28cabc59ee713f14baf50f2efbb20f2910ed46047b4
werewolfd_sha256=a2cdb36989c45aa9ccccdbb7e431b7e7851080c4cb8736e81f6ec8d5633788b8
werewolfctl_sha256=c9c5e5cf17350b675927efd419e7bb970c3e136ff79977ce8101357e07f74818
archive_sha256=155b4df116bcd02db2ef0388314cc6745babe90b05b66cbff761471997ec2f0e
```

Two clean target-directory builds produced byte-identical daemon and control
binaries. Two independently packaged archives using the same source commit,
toolchain, and `SOURCE_DATE_EPOCH` were also byte-identical. The archive
SHA-256 above is shared by both. `SHA256SUMS` validated every packaged member.
The archive contains only `INSTALL.md`, `RELEASE-METADATA`, `SHA256SUMS`,
`bin/werewolfd`, `bin/werewolfctl`, `install.sh`,
`systemd/werewolfd.service`, and `uninstall.sh`.

`ldd` reports runtime dependencies on the normal x86_64 loader, `libc.so.6`,
`libm.so.6`, and `libgcc_s.so.1`; the binaries are not claimed to be static.
No Rust dependency changed. The release outputs include no test Dens, keys,
traffic secrets, payload captures, debug/test binaries, temporary logs, or
developer-home paths.

## Fault and environment boundaries

Direct Stage20 actions were private loopback peer stop/restart, active
forwarding, SIGTERM/SIGKILL recovery, target and Pack revocation, Silver,
port collision, malformed TCP/UDP input, saturation, stream churn, and
administrative mutation. The harness uses a hard timeout per operation and
removes its temporary topology on completion. The final temporary parent had
no remaining `stage20-*` fixture directories.

The environment did not provide `tc`; no netem packet loss, jitter, or
reordering profile was run. Stage20 did not mount read-only filesystems,
synthesize ENOSPC, change the host clock, or run systemd restart chaos. Stage15B
storage fault tests, Stage16 resource/deadline tests, Stage17 lifecycle and
systemd tests, and Stage19 installed-artifact tests were rerun where listed,
but are not represented as direct Stage20 chaos injection. No persistent core
dumps were enabled.

Allocator-cached RSS can remain above startup after load; the measured
criterion is no observed progressive growth across the tested cycles, not
RSS returning byte-for-byte to baseline. CPU, kernel buffers, resolver,
network infrastructure, distributed floods, and privileged-host failures
remain outside this certification. Werewolf does not claim DoS immunity.

## Claims and verdict

| Claim | Result | Evidence / boundary |
|---|---|---|
| C1 TCP 1,000-cycle authenticated reconnect | CERTIFIED | Three corrected/final extended runs each completed 1,000; long loops added 83,280 and 81,563 cycles. |
| C2 QUIC 1,000-cycle authenticated reconnect | CERTIFIED | Same per-run evidence; 10,000-route single-Fang regression also passes. |
| C3 transport alternation has no stale cross-transport state | CERTIFIED | 1,000 alternations per extended run and final packaged CI. |
| C4 STRICT never falls back to plaintext | CERTIFIED | Fallback matrix regressions and secure-only Stage20 profiles fail closed. |
| C5 loss/blackhole does not permanently poison service | CERTIFIED | Loopback peer loss, restart, malformed/saturation recovery and later successful connections; netem was unavailable. |
| C6 peer restart recovers safely | CERTIFIED | 100 single-peer and 10 both-peer restarts in each long run. |
| C7 SIGKILL preserves state and safe restart | CERTIFIED | 100 phase-varied SIGKILL recoveries in each long run; Stage17 stale lock/socket tests pass. |
| C8 Silver remains effective on active/establishing workloads | CERTIFIED | Updated long harness holds TCP/QUIC routes through Silver fencing; Stage11C establishment tests pass. |
| C9 peer revocation remains effective on active/establishing workloads | CERTIFIED | Updated harness checks live TCP/QUIC routes and post-removal denial; Stage11C authority tests pass. |
| C10 target revocation leaves no stale authorization | CERTIFIED | 20 remove/deny/restore cycles per long run and final CLI regressions. |
| C11 Fang/admin churn preserves consistency | CERTIFIED | 20 lifecycle cycles per long run; 40 generation mutations and crash recovery. |
| C12 saturation releases reusable capacity | CERTIFIED | Pre-auth and authority saturation repeated; legitimate reconnect succeeds after release. |
| C13 malformed traffic causes no panic or demonstrated unbounded growth | CERTIFIED | Repeated malformed input, bounded logs/resources, post-flood reconnect, no daemon panic. |
| C14 no demonstrated FD leak | CERTIFIED | Long-run FDs return to 22 baseline after 24 peak. |
| C15 no demonstrated workload-correlated unbounded RSS growth | CERTIFIED | Two long runs show bounded post-load RSS; allocator retention is not interpreted as a leak. |
| C16 daemon returns to quiescent CPU | CERTIFIED | Final one-second CPU tick delta is zero in both long runs and final CI. |
| C17 tested Den/runtime state stays bounded | CERTIFIED | Den grew by 595 and 1,480 bytes in the final long runs; runtime sockets/locks recover and fixtures are removed. |
| C18 local administration remains responsive under load | CERTIFIED | 1,000 control operations and serialized concurrent mutations per long run. |
| C19 large transfers have zero corruption | CERTIFIED | 30 TCP and 30 QUIC 50 MiB transfers across three extended campaigns, plus final packaged CI transfer per transport. |
| C20 all found defects resolved with regressions | CERTIFIED | EOF lease fix, reusable QUIC endpoint, completed-handle pruning, targeted regressions, and passing final gates. |
| C21 no wire-format change | CERTIFIED | No Stage20 protocol source changes; valid sessions keep Stage10 behavior. |
| C22 no persistence-format change | CERTIFIED | Stage15B files unchanged; generation/crash tests pass. |
| C23 no CLI-surface change | CERTIFIED | No CLI implementation changes; Stage18 tests pass. |
| C24 no dependency change | CERTIFIED | Cargo.lock SHA unchanged. |
| C25 Stage9–19 guarantees remain intact | CERTIFIED | Full workspace/Python/acceptance and Stage17–19 release harnesses pass. |
| C26 duration and residual denial-of-service risks reported | CERTIFIED | More than 4.4 hours recorded; distributed/resource-external DoS is not claimed solved. |

**STAGE20_WIRE_FORMAT_CHANGE = NONE**

**STAGE20_PERSISTENCE_FORMAT_CHANGE = NONE**

**STAGE20_CLI_SURFACE_CHANGE = NONE**
**STAGE20_SECRET_MATERIAL_AUDIT = PASS**

**STAGE20_ADVERSARIAL_SOAK = CERTIFIED**

This certification records tested behavior and bounded resource recovery; it
does not claim immunity to arbitrary denial of service or failure outside the
tested Linux x86_64 environment.
