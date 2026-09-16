# WerewolfProxy Stage 0 baseline

Recorded on 2026-09-07. Scope: the existing working tree, without production-code changes or defect fixes. This is an observed local baseline, not a claim of security assurance, cross-host interoperability, release-build validation, or seamless migration of established connections.

## Result

All seven required acceptance areas passed against `target/debug/werewolfd` built from this checkout. Control operations used `target/debug/werewolfctl`; selection used the unchanged checkout `scripts/wolf-b.sh auto --policy secure --json` with supported URL overrides.

The tests ran in isolated Den copies with remapped loopback ports. The installed A/B services and timers remained running and were not restarted, stopped, reconfigured or replaced. Their executable hashes, PIDs and persistent configuration were checked separately. No existing dirty file was changed.

| Acceptance area | Result actually executed in Stage 0 | Evidence |
|---|---|---|
| QUIC application traffic | PASS: HTTP GET returned the exact 51-byte test marker | `quic-traffic.stdout`, `results.json` |
| TCP encrypted v2 application traffic | PASS: same exact application marker through transport `tcp` | `encrypted-traffic.stdout` |
| TCP plain application traffic | PASS: same marker through transport `tcp-plain` | `plain-traffic.stdout` |
| Secure fallback sequence | PASS: QUIC -> TCP encrypted v2 -> TCP plain, closing the respective isolated Fang listeners between selections; GET through every selected URL succeeded | `secure-initial-quic.*`, `secure-fallback-encrypted.*`, `secure-fallback-plain.*` |
| Restoration to QUIC | PASS: reopened the encrypted-TCP and QUIC profiles using checkout CLI; selector returned QUIC and GET succeeded | `restore-home-web-*.stdout`, `secure-restored-quic.*` |
| Encrypted TCP 50 MiB integrity | PASS: exactly 52,428,800 bytes received; source and destination SHA256 equal; curl exit 0; command elapsed 20.121 seconds | `encrypted-50mib.*`, `results.json` |
| Daemon restart/profile restoration | PASS: terminated both isolated daemons with SIGTERM, relaunched the same binaries and Den copies without manually reopening Fangs; all four listeners restored with new IDs; active-profile list unchanged across restart; GETs passed on all three transports and HEAD passed on large-transfer Fang | `before-restart.json`, `after-restart.json`, `restart-*`, `secure-after-restart.*` |

The integrity hash was:

```text
80d160f35ab0b90c95b2f4777c0fa0127bb2a4b5f6fff2f4a6c0f3a4c1219ea6
```

Additional checks executed successfully:

```bash
cargo build --locked --offline --workspace --bins
cargo test --locked --offline --workspace
bash scripts/script-gate.sh
bash scripts/repo-gate.sh
```

The Rust test command passed 16 core tests (8 unit and 8 integration tests). The daemon, CLI and lab binary test targets contain no tests. These results do not substitute for the application acceptance run.

The existing RC/chaos/large-transfer/recovery gate scripts were not run verbatim: they use fixed live sockets, profiles, target paths, systemd units or installed binaries. Stage 0 reproduced their required acceptance operations in an isolated one-shot runner instead. The `auto` policy implementation itself was executed unchanged. This run does not validate the watchdog, the entire `heal` command, systemd restart integration, reinstall recovery, arbitrary service gates, or other policies.

## Source identity and dependency state

Repository: `/home/sleepdeprivedloon/Projects/werewolfproxy`

Git HEAD:

```text
63691aade2aa165b16864a80272f2a55eadcee00
```

Initial working-tree status:

```text
 M Cargo.lock
 M crates/werewolf-core/src/peer_store.rs
 M crates/werewolfd/src/main.rs
```

| Existing dirty file | SHA256 of tested contents |
|---|---|
| `Cargo.lock` | `4a40da9c4faade9b7fe4424a4c7419ce81b546c9fd34fd8cac97bcb5c22db51d` |
| `crates/werewolf-core/src/peer_store.rs` | `994a88774f1f2c984d9bfe5ed599183b4d88bce4143c6493e4642ea5f504c9d3` |
| `crates/werewolfd/src/main.rs` | `424b7988a4f3f71e2d622b3fbe4ab2f4b4e9fef70adb7bea748e68a981f83eee` |

The commit alone is insufficient to reproduce the tested source. The full `git diff --binary` is retained as `working-tree.patch`, SHA256 `88432c396aee597cc4ff6b1da2d9b8da5222602416c2c882bb73d1e03ab01f06`. The patch was byte-identical before and after testing. `source-hashes-before.json` records every non-build, non-Git file present at capture. No captured source file changed during testing.

`Cargo.lock` uses format 4 and contains 222 package entries. Both Cargo commands used `--locked --offline`, succeeded using already available dependencies, and left the lockfile unchanged. No dependency update was performed. Representative locked versions: Tokio 1.52.3, Quinn 0.11.9, rustls 0.23.40, rcgen 0.13.2, ed25519-dalek 2.2.0, x25519-dalek 2.0.1, chacha20poly1305 0.10.1, BLAKE3 1.8.5, sha2 0.10.9, serde_json 1.0.149, tempfile 3.27.0. The lockfile, not this abbreviated list, is authoritative.

| Component | Recorded version |
|---|---|
| OS | Debian GNU/Linux 13 (trixie), 13.6, x86_64 |
| Kernel / libc | Linux 6.12.107+deb13-amd64 / glibc 2.41 |
| Active Rust toolchain | `stable-x86_64-unknown-linux-gnu` (default), resolving to 1.96.0 at capture |
| rustc | `1.96.0 (ac68faa20 2026-05-25)`; full commit `ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96` |
| Rust host / LLVM | `x86_64-unknown-linux-gnu` / 22.1.2 |
| Cargo | `1.96.0 (30a34c682 2026-05-25)` |
| Python | 3.13.5 |
| curl | 8.14.1 |
| jq | 1.7 |
| Bash | 5.2.37(1)-release |

No `CARGO_*`, `RUST*` or `WEREWOLF_*` environment overrides were present in the inventory shell. Its PATH was:

```text
/home/sleepdeprivedloon/.cargo/bin:/home/sleepdeprivedloon/.local/bin:/usr/local/bin:/usr/bin:/bin:/usr/local/games:/usr/games
```

The `stable` name can move; verify the exact rustc version before reproducing. This baseline exercised the debug/dev build, not a new release build. Rebuilding with another toolchain or build environment may change binary hashes even when functional tests pass.

## Checkout versus installed executables

| Kind | Absolute path | SHA256 |
|---|---|---|
| Tested checkout daemon | `/home/sleepdeprivedloon/Projects/werewolfproxy/target/debug/werewolfd` | `7008d0a039b7b14c1ce79d70c7fb97796aefc24fea1223633e07f8f236e0796c` |
| Tested checkout CLI | `/home/sleepdeprivedloon/Projects/werewolfproxy/target/debug/werewolfctl` | `04747dd50a5aec59ea9a9e51392b467f92aa9238e829694f3b7c2af7711a234e` |
| Installed/live daemon | `/home/sleepdeprivedloon/.local/bin/werewolfd` | `45d4ebdc6e7e74947e42c8dcf91c1e27c515c67cad8344b3fdddea76820cb0a6` |
| Installed CLI | `/home/sleepdeprivedloon/.local/bin/werewolfctl` | `bd1cd7df6776cc82598a5c40d34f52c8cc63ac02a34d952f57275b57a9c62171` |

The isolated A/B process executable paths and hashes were verified through `/proc/<pid>/exe` (first generation PIDs 11852/11860). Live A/B PIDs 1232/1236 remained unchanged and both matched the installed daemon hash after testing. The installed CLI was used only for version and live `den info` inventory, not for checkout acceptance.

Installed `wolf-a`, `wolf-b`, `wolf-c`, and `wolf-d` are regular files in `~/.local/bin` and were byte-identical to their checkout scripts. This does not imply that the installed Rust binaries match the checkout.

Installed CLI version text reports commit `83c4871`, built `2026-06-05 13:55:59 UTC`. Checkout CLI reports unknown commit/build metadata. Both print `profile: release`, but that text is hardcoded and does not identify the actual checkout build profile. Hashes, build commands and process paths establish provenance.

### Executables exercised by existing gates

The following is the reviewed call/resolution map in the recorded environment, not a claim that every gate was executed. "Installed" means the paths/hashes above. A curl probe exercises whichever process owns its target port; `cargo build` does not replace an already running daemon.

| Existing gate/check | Rust executables and runtime selected by the existing script | Stage 0 execution |
|---|---|---|
| `repo-gate.sh`, `script-gate.sh` | No Rust daemon/CLI; Git, find, Bash and related tools | Executed: PASS |
| `quick-gate.sh` | Builds checkout debug binaries; `wolf-b` resolves installed wrapper/CLI and fixed live socket; probes live ports | Not executed as a whole |
| `rc-gate.sh` | Builds checkout; uses checkout A/B wrappers, but explicitly prepends `~/.local/bin`, so control resolves installed CLI; checks live systemd services | Not executed |
| `json-contract-gate.sh` | Installed `wolf-b`; nested doctor/ready/selftest can access live sockets, heal and probe live ports | Not executed |
| `schema-gate.sh` | Same shell runtime plus checkout dashboard/peer-contract producers and `contracts/` files | Not executed |
| `dashboard-gate.sh` | Checkout dashboard script -> installed `wolf-b` -> live runtime/probes | Not executed |
| `chaos-gate.sh`, `chaos-gate-extended.sh` | Installed `wolf-b`/CLI; close/reopen Fangs on `/tmp/wolf-b.sock`; probe fixed or overridden URLs | Not executed verbatim; required selection chain reproduced in isolation |
| `soak-gate.sh` | Installed `wolf-b`/CLI; live healing, scores, snapshots and optional Fang closure; target helper starts Python services | Not executed |
| `dev-gate.sh` | Installed wrapper selftest, then extended chaos and RC dependencies above | Not executed |
| `release-gate.sh` | Checkout build, installed wrapper JSON/ready/snapshot, RC dependencies above | Not executed |
| `full-gate.sh` | Quick/JSON/dashboard/peer/peer-Rust/RC/chaos/soak compositions; mixes checkout builds and installed daemons | Not executed |
| `nightly-gate.sh` | Release/extended-chaos/soak composition, same mixed provenance | Not executed |
| `multiwolf-gate.sh` | `start-multiwolf.sh` launches PATH-resolved installed `werewolfd` for A-D; installed control wrappers | Not executed |
| `peer-rust-gate.sh` | Starts installed A-D daemons and probes installed CLIs; `cargo run ... peer_lab` executes checkout debug `peer_lab` | Not executed |
| `peer-gate.sh`, `peer-export-gate.sh` | Shell/jq file operations on shell peer DB; no daemon required by these gate bodies | Not executed |
| `service-gate.sh` | Starts installed multiwolf daemons; peer health probes invoke installed CLIs; writes shell DBs | Not executed |
| `service-http-gate.sh`, `service-tcp-gate.sh`, `service-ssh-gate.sh` | Installed multiwolf daemons and `wolf-b`/CLI; QUIC default for service-created Fang; Python HTTP/TCP or existing SSH target | Not executed |
| `dev-check.sh` | Builds checkout; stops A/B processes; starts checkout daemons through `cargo run`; smoke control still resolves installed CLI | Not executed: live systemd services make its reset/start flow unsuitable |
| `smoke-test.sh` | Checkout wrappers resolve installed CLI; daemon provenance depends on current A/B socket owners | Not executed |
| `release-check.sh`, `final-rc.sh` | `dev-check` chain plus curl and archive creation; not a pure installed or checkout-only gate | Not executed |
| `recovery-check.sh` | Restarts live systemd services using installed daemon, then quick/chaos dependencies | Not executed |
| `watchdog-check.sh` | Reinstalls timer/unit and runs installed wrapper healing against live daemons | Not executed |
| `tcp-v2-large-test.sh` | curl/SHA256 only; URL owner is the exercised daemon; fixed source file and direct source port 8081 | Not executed verbatim; equivalent 50 MiB transfer/hash check passed on isolated checkout Fang |
| `transport-chaos.sh`, `chaos-curl.sh`, `tunnel-health.sh` | Installed control/live watchdog for transport-chaos; curl endpoint owners for traffic probes | Not executed |
| `gate-report.sh` | Dispatches quick/release/full/nightly plus installed wrapper reports | Not executed |
| Hook produced by `install-git-hooks.sh` | Checkout build plus installed wrapper and quick/JSON/schema dependencies | Not installed or executed |

Changing PATH alone is insufficient to isolate these gates: sockets, units, healing ports and some target paths remain fixed, and RC prepends the installed binary directory.

## Den directories, identities and Pack

Live Den roots are `~/.config/wolf-a`, `wolf-b`, `wolf-c`, and `wolf-d`. `~/.config/werewolf` was absent. A/B were running; C/D were configured but had no running daemon in the observed process/port inventory.

The tested Den roots were `/tmp/werewolf-stage0-20260907/wolf-a` and `/tmp/werewolf-stage0-20260907/wolf-b`; Unix sockets were sibling `wolf-a.sock` and `wolf-b.sock` under that private directory. Den directory mode was 0700. Pelt files were copied with their original 0600 permissions. No identity was regenerated or initialized, and no private key is included in this document or the public evidence bundle.

| Wolf | Pelt fingerprint | Public key (Base64) |
|---|---|---|
| A | `wwp1:4E-F9-FC-B8-F6-C6-6B-04` | `g70IYN4nLXGAPa/lSNeZQFE9ID9VDzDHK+sAf+nbd5Y=` |
| B | `wwp1:47-32-27-4C-6F-DF-32-E8` | `HGZ/XluE4nztYw1UrqYX7t2zKn2Z086aEVUsf2PwFs4=` |
| C | `wwp1:C6-45-71-A9-40-9E-64-E4` | `bSTLSYRU3B4E/dTXCmkcB2vkZhWjnjhGV3PyEBTtfTc=` |
| D | `wwp1:18-49-13-7B-77-16-A0-58` | `J9bA52iD9WolEdBIYEdDCa3PORV0k1XNcC43nNdiLIw=` |

The isolated lab used the same A/B Pelt files and fingerprints as the host. All Pack records have `trust: "Packmate"`:

| Den | Pack name | Identity | Live address | Public key stored? |
|---|---|---|---|---|
| A | wolf-b | B | `quic://127.0.0.1:9561` | B key |
| A | wolf-c | C | `quic://127.0.0.1:9562` | null |
| A | wolf-d | D | `quic://127.0.0.1:9563` | null |
| B | wolf-a | A | `quic://127.0.0.1:9560` | A key |
| B | wolf-a-tcp | A | `tcp://127.0.0.1:8443` | A key |
| B | wolf-c | C | `quic://127.0.0.1:9562` | null |
| B | wolf-d | D | `quic://127.0.0.1:9563` | null |
| C | wolf-a / wolf-b / wolf-d | A / B / D | QUIC ports 9560 / 9561 / 9563 | all null |
| D | wolf-a / wolf-b / wolf-c | A / B / C | QUIC ports 9560 / 9561 / 9562 | all null |

The test copied A/B Pack arrays and changed only A/B endpoint ports using the mapping below. Unused C/D entries remained unchanged and were not exercised. The `wolf-a` and `wolf-a-tcp` aliases intentionally share A's identity and key. Recreating these through `pack.add` would fail its duplicate-fingerprint check and would omit public keys; the runner therefore copies existing configuration files, using the unchanged file formats accepted by daemon loading.

Live A has `pelt.json` and `pack.json` only. Live B also has `fangs.json`, `active_fangs.json`, `peers.json` and `services.json`. C/D have Pelt and Pack files only. The shell `peers.json` and `services.json` were not inputs to the acceptance run. Its peer telemetry is dated June 2026 and is not current health evidence; notably its `wolf-a-tcp` address is 9443, unlike Pack's 8443. This inconsistency was not changed.

## Fang profiles and port map

All addresses below use loopback. Each profile lives in B's `fangs.json`:

| Profile | Peer | Transport field | Live local -> remote | Isolated local -> remote |
|---|---|---|---|---|
| `home-web-quic` | `wolf-a` | `quic` | 9020 -> 8080 | 19020 -> 18080 |
| `home-web-tcp` | `wolf-a-tcp` | `tcp-plain` | 9021 -> 8080 | 19021 -> 18080 |
| `home-web-tcp-encrypted-v2` | `wolf-a-tcp` | `tcp` | 9022 -> 8080 | 19022 -> 18080 |
| `large-transfer-tcp-v2` | `wolf-a-tcp` | `tcp` | 9032 -> 8081 | 19032 -> 18081 |

`tcp-encrypted-v2` is the selector label; the daemon/profile configuration value remains `tcp`. Plain TCP reaches the target directly from B; it does not traverse A's TCP listener. These behaviors were not altered.

| Purpose | Live/default port | Isolated port | Protocol |
|---|---|---|---|
| A peer listener | 8443 | 18443 | TCP encrypted |
| B peer listener | 9443 | 19443 | TCP encrypted |
| A peer listener | 9560 | 19560 | QUIC/UDP |
| B peer listener | 9561 | 19561 | QUIC/UDP |
| C peer listeners, unused | 10443 / 9562 | not started | TCP / UDP |
| D peer listeners, unused | 11443 / 9563 | not started | TCP / UDP |
| B QUIC Fang local listener | 9020 | 19020 | TCP |
| B plain Fang local listener | 9021 | 19021 | TCP |
| B encrypted Fang local listener | 9022 | 19022 | TCP |
| B large-transfer Fang local listener | 9032 | 19032 | TCP |
| HTTP application target | 8080 | 18080 | TCP |
| HTTP large-file target | 8081 | 18081 | TCP |

Initial live and cloned `active_fangs.json` order:

```json
["large-transfer-tcp-v2", "home-web-quic", "home-web-tcp-encrypted-v2", "home-web-tcp"]
```

Closing/reopening profiles in the isolated test changed only the isolated list order to:

```json
["large-transfer-tcp-v2", "home-web-tcp", "home-web-tcp-encrypted-v2", "home-web-quic"]
```

That JSON list was unchanged across the restart check. All four new Fang IDs differed from the pre-restart IDs. The host list and other host JSON files matched their original hashes after testing. Live `active_fangs.json` mode was 0664; Pack/Pelt/profile files were 0600. These permissions were observed, not repaired.

## Supervision and required services

Loaded unit definitions had no drop-ins. Unit files are in `~/.config/systemd/user`:

| Unit | Observed state | Relevant configuration |
|---|---|---|
| `werewolf-a.service` | enabled, active/running, PID 1232 | Installed daemon; `/tmp/wolf-a.sock`; A Den; TCP 8443, QUIC 9560 |
| `werewolf-b.service` | enabled, active/running, PID 1236 | Installed daemon; `/tmp/wolf-b.sock`; B Den; TCP 9443, QUIC 9561 |
| `werewolf-b-watchdog.timer` | enabled, active/waiting | `OnBootSec=30`, `OnUnitActiveSec=30`, `AccuracySec=5` |
| `werewolf-b-watchdog.service` | static, inactive between runs | `/bin/bash -lc "%h/.local/bin/wolf-b heal --quiet || true"` |
| `werewolf-b-maintenance.timer` | enabled, active/waiting | Daily midnight, persistent, ten-minute accuracy |
| `werewolf-b-maintenance.service` | static, inactive at inspection | `/bin/bash -lc "%h/.local/bin/wolf-b maintenance"` |
| `werewolf-peer-monitor.service` | not found | Repository installer exists; no loaded monitor on this host |

A/B service settings: `Restart=on-failure`, `RestartSec=2`, `KillMode=control-group`, `TimeoutStopSec=5`, and `ExecStartPre=/usr/bin/rm -f /tmp/wolf-<a|b>.sock`. The executable paths are absolute installed paths. A process-level restart test does not establish systemd recovery.

The watchdog can undo failure injection on live Fangs. Maintenance/status/ready commands can also invoke healing or write reports. No existing units were touched. Isolated socket paths and ports prevented those services from controlling the test daemons.

At host inventory time, live Werewolf peer and Fang listeners existed, but no target service was listening on 8080 or 8081. An SSH service existed on 22. Target/service prerequisites for the unmodified broader gates include:

- `test-targets.sh`: Python HTTP service serving the repository on 8080 and `/tmp/werewolf-bigtest/large.bin` on 8081, nominally 50 MiB.
- `service-http-gate.sh`: repository directory listing through local 18082 (default).
- `service-tcp-gate.sh`: generated Python TCP responder on 19090 through local 19091 (defaults).
- `service-ssh-gate.sh`: SSH banner on target 22 through local 12222; absence skips the gate.
- Four configured/running wolves for the multiwolf and peer-Rust gate family, with wrapper commands installed. Existing C/D null public keys are not sufficient evidence of working signed QUIC traffic.

The executed acceptance lab instead started two explicit loopback-only Python HTTP services:

```bash
python3 -m http.server 18080 --bind 127.0.0.1 --directory "$BASE/target"
python3 -m http.server 18081 --bind 127.0.0.1 --directory "$BASE/large-target"
```

The application file `baseline.txt` was exactly `WerewolfProxy checkout Stage 0 application payload\n`. The 50 MiB file was deterministic: concatenate SHA256 of every four-byte big-endian integer from 0 through 32767 to make one MiB, then repeat that block 50 times. This is a transfer-integrity fixture, not cryptographic key material or a throughput benchmark. The legacy large-test script's random-source creation and hardcoded source check were not executed.

## Reproduction commands

These commands reproduce the isolated lab, not the host's systemd lab. They require the recorded source state, exact Rust version, available locked dependencies, existing A/B private Pelt files and configuration, Python, curl, jq, Bash, and free ports from the isolated map. Run with permission to bind/connect loopback and Unix sockets; the Codex sandbox required explicit host execution for network tests.

Private keys are intentionally not embedded. To replay on this host, use the existing A/B files only after checking the recorded hashes. For a different machine, transfer authorized configuration privately or create a separately documented test-identity baseline; copying just public keys cannot reproduce these identities.

```bash
cd /home/sleepdeprivedloon/Projects/werewolfproxy
EVIDENCE="$HOME/.local/state/werewolfproxy/baselines/2026-09-07-stage0"

test "$(git rev-parse HEAD)" = 63691aade2aa165b16864a80272f2a55eadcee00
test "$(rustc -V)" = 'rustc 1.96.0 (ac68faa20 2026-05-25)'

# Check all captured source files and host JSON inputs against the run inventory.
python3 - "$EVIDENCE" <<'PY'
import hashlib, json, pathlib, sys
base = pathlib.Path(sys.argv[1])
source = json.loads((base / 'source-hashes-before.json').read_text())
for name, expected in source.items():
    assert hashlib.sha256(pathlib.Path(name).read_bytes()).hexdigest() == expected, name
inventory = json.loads((base / 'inventory.json').read_text())
for name, expected in inventory['live_config_hashes'].items():
    assert hashlib.sha256(pathlib.Path(name).read_bytes()).hexdigest() == expected, name
PY

cargo build --locked --offline --workspace --bins
cargo test --locked --offline --workspace
bash scripts/script-gate.sh
bash scripts/repo-gate.sh

# A fresh private directory prevents overwriting evidence or reusing modified test profiles.
BASE="$(mktemp -d /tmp/werewolf-stage0-replay.XXXXXX)"
cp "$EVIDENCE/run-baseline.py" "$BASE/run-baseline.py"
python3 "$BASE/run-baseline.py"
jq . "$BASE/results.json"
jq . "$BASE/process-cleanup.json"
```

The runner is reproduced in full in the appendix, so its code is not dependent on `/tmp` surviving. The durable evidence directory contains the tested dirty-source patch and source hash manifest; copy those with the baseline if another checkout must reproduce this exact dirty state. Do not apply that patch over an already dirty checkout. The source patch does not contain private Pelt files.

Underlying daemon commands used by the runner (with the recorded test `BASE`) were:

```bash
./target/debug/werewolfd --socket "$BASE/wolf-a.sock" --home "$BASE/wolf-a" \
  --listen 127.0.0.1:18443 --quic-listen 127.0.0.1:19560
./target/debug/werewolfd --socket "$BASE/wolf-b.sock" --home "$BASE/wolf-b" \
  --listen 127.0.0.1:19443 --quic-listen 127.0.0.1:19561
```

Selection used these overrides without editing the script or exporting a replacement HOME/PATH:

```bash
NO_PROXY='*' no_proxy='*' \
WEREWOLF_TARGET_URL=http://127.0.0.1:18080/baseline.txt \
WEREWOLF_QUIC_URL=http://127.0.0.1:19020/baseline.txt \
WEREWOLF_TCP_ENC_URL=http://127.0.0.1:19022/baseline.txt \
WEREWOLF_TCP_URL=http://127.0.0.1:19021/baseline.txt \
bash scripts/wolf-b.sh auto --policy secure --json
```

Every successful selection was followed by curl GET and exact body comparison. Fault injection used `./target/debug/werewolfctl --socket "$BASE/wolf-b.sock" fang close <id>` for the Fang on 19020, then 19022. Restoration used `fang open-profile home-web-tcp-encrypted-v2` and `fang open-profile home-web-quic`. Restart used SIGTERM/wait on the runner's own A/B PIDs, then the same daemon arguments, with no `fang open-profile` after restart. `commands.json` records the actual argv, dynamically discovered Fang IDs and command exit statuses.

## Evidence, historical material and limits

Original execution/evidence directory: `/tmp/werewolf-stage0-20260907`.

Durable evidence: `/home/sleepdeprivedloon/.local/state/werewolfproxy/baselines/2026-09-07-stage0`.

The durable copy includes public fixture inventory, source manifest, dirty-source patch, runner, results, command transcripts, daemon/target logs, unit inventory, executable information, cleanup status and unchanged-file checks. It excludes copied private Den directories and large payload/download files. The original private test directory still contains the copied Pelt files and payloads; do not publish that directory wholesale.

| Artifact | Purpose |
|---|---|
| `inventory.json` | Source identity, toolchain, executable hashes, all original live JSON file hashes |
| `source-hashes-before.json`, `working-tree.patch` | Exact tested working-tree reconstruction/checks |
| `fixtures-public.json` | Exact remapped Pack arrays, profiles, initial active names, public identities, ports and payload hash |
| `commands.json`, `results.json` | Actual execution and assertions |
| `wolf-a-1.log`, `wolf-b-1.log`, `wolf-a-2.log`, `wolf-b-2.log` | First-start and restart transport logs |
| `host-units.txt`, `host-ports-after.txt`, `live-control-inventory.json` | Installed/live environment, distinct from tested daemons |
| `process-cleanup.json` | All six spawned daemon/target processes exited via SIGTERM |
| `unchanged-check.json` | No captured source or live JSON file changes; exact original dirty patch retained |
| `script-gate.sh.stdout`, `repo-gate.sh.stdout` | Existing gates actually run |

Cargo output was observed in the Stage 0 tool transcript; no standalone raw Cargo log was retained. No remote upload, install, unit edit or protocol change occurred.

Historical evidence is separate: `TRANSPORT_ARCHITECTURE.md` and `docs/STATUS.md` state earlier validation of the three-transport stack, integrity and recovery. The four `werewolf_support_20260526_*.tar.gz` files contain older QUIC diagnostics, Pack/profile samples and restoration logs with different identities. `werewolfproxy_authenticated_fang_v1.tar.gz` and `werewolfproxy_rc1_silver_logs_20260524_1735.tar.gz` are empty archives. None is counted as a Stage 0 test pass.

Blocking issues for the seven isolated acceptance checks: **none**. Constraints on stronger claims remain: host fixed-port targets were absent; existing composite gates mix installed binaries and live state; watchdog/systemd/reinstall behavior was not retested; private key files and the dirty-source patch are prerequisites for exact replay. A passing local baseline does not correct or negate previously identified authentication, cancellation, framing, persistence or reporting defects.

## Appendix: exact one-shot runner executed in Stage 0

Runner SHA256: `39d09594c02a208daaeb7269657e99933b7f4ab500f02514c0e17d5e73486a81`.

Save the following block as `run-baseline.py` in a fresh private directory, then run it from the recorded host with the checkout built. It creates only test files/processes outside the repository, reads live configuration, operates only on copied Den state, and stops its own processes. Its hardcoded repository path reflects the recorded environment. This is Stage 0 reproduction material, not a new production module or general test-framework refactor.

```python
#!/usr/bin/env python3
"""Stage 0 one-shot acceptance run; no changes to repository or live services."""
import datetime, hashlib, json, os, pathlib, shutil, socket, subprocess, sys, time
ROOT = pathlib.Path('/home/sleepdeprivedloon/Projects/werewolfproxy')
BASE = pathlib.Path(__file__).resolve().parent
BIN = ROOT / 'target/debug'
PORTS = {'a_tcp':18443,'b_tcp':19443,'a_quic':19560,'b_quic':19561,'quic':19020,'plain':19021,'encrypted':19022,'large':19032,'target':18080,'large_target':18081}
REMAP = {8443:18443,9443:19443,9560:19560,9561:19561,9020:19020,9021:19021,9022:19022,9032:19032,8080:18080,8081:18081}
MARKER = b'WerewolfProxy checkout Stage 0 application payload\n'
processes=[]
handles=[]
results=[]
commands=[]

def write_json(path, obj):
    path.write_text(json.dumps(obj,indent=2)+'\n')

def command(args, label, env=None, timeout=30):
    start=time.monotonic()
    try:
        r=subprocess.run([str(x) for x in args],cwd=ROOT,env=env,capture_output=True,timeout=timeout)
        rc,out,err=r.returncode,r.stdout,r.stderr
    except subprocess.TimeoutExpired as e:
        rc,out,err=124,e.stdout or b'',e.stderr or b''
    (BASE/(label+'.stdout')).write_bytes(out)
    (BASE/(label+'.stderr')).write_bytes(err)
    commands.append({'label':label,'argv':[str(x) for x in args],'exit':rc,'seconds':round(time.monotonic()-start,3)})
    write_json(BASE/'commands.json',commands)
    return rc,out,err

def record(name,ok,detail):
    row={'test':name,'status':'PASS' if ok else 'FAIL','detail':detail}
    results.append(row)
    write_json(BASE/'results.json',results)
    print(json.dumps(row),flush=True)

def start_process(args,label):
    h=open(BASE/(label+'.log'),'ab');handles.append(h)
    p=subprocess.Popen([str(x) for x in args],cwd=ROOT,stdout=h,stderr=subprocess.STDOUT)
    processes.append(p)
    commands.append({'label':label,'argv':[str(x) for x in args],'pid':p.pid})
    write_json(BASE/'commands.json',commands)
    return p

def start_wolf(wolf, generation):
    p=start_process([BIN/'werewolfd','--socket',BASE/(wolf+'.sock'),'--home',BASE/wolf,'--listen','127.0.0.1:'+str(PORTS[wolf[-1]+'_tcp']),'--quic-listen','127.0.0.1:'+str(PORTS[wolf[-1]+'_quic'])],wolf+'-'+str(generation))
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        if p.poll() is not None:raise RuntimeError(wolf+' exited during startup')
        try:
            control(wolf,'status',{},wolf+'-startup-'+str(generation));break
        except (OSError,ValueError):time.sleep(.1)
    else:raise RuntimeError(wolf+' control socket unavailable')
    time.sleep(1)
    return p

def stop_process(p):
    if p.poll() is None:
        p.terminate()
        try:p.wait(timeout=5)
        except subprocess.TimeoutExpired:p.kill();p.wait(timeout=5)

def control(wolf,cmd,args,label):
    with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as s:
        s.settimeout(5);s.connect(str(BASE/(wolf+'.sock')))
        s.sendall((json.dumps({'id':'stage0','cmd':cmd,'args':args})+'\n').encode())
        data=b''
        while not data.endswith(b'\n'):
            chunk=s.recv(65536)
            if not chunk:raise RuntimeError('control EOF')
            data+=chunk
    value=json.loads(data)
    write_json(BASE/(label+'.json'),value)
    return value

def get_marker(url,label):
    rc,out,err=command(['curl','--noproxy','*','--fail','--silent','--show-error','--max-time','12',url],label,timeout=15)
    return rc==0 and out==MARKER,{'url':url,'curl_exit':rc,'bytes':len(out),'exact_payload':out==MARKER,'error':err.decode(errors='replace')}

def choice(expected,label,env):
    rc,out,err=command(['bash',ROOT/'scripts/wolf-b.sh','auto','--policy','secure','--json'],label,env=env,timeout=30)
    try:value=json.loads(out)
    except ValueError:value={}
    ok=rc==0 and value.get('transport')==expected and value.get('healthy') is True
    traffic=False
    if value.get('url'):
        traffic,detail=get_marker(value['url'],label+'-application')
    else:detail={'error':'no selected URL'}
    record(label,ok and traffic,{'selection':value,'selector_exit':rc,'application':detail})

def close_port(port,label):
    response=control('wolf-b','fang.list',{},label+'-before')
    matches=[f for f in response['result'] if f['local']=='127.0.0.1:'+str(port)]
    if len(matches)!=1:raise RuntimeError('expected exactly one Fang on '+str(port))
    rc,out,err=command([BIN/'werewolfctl','--socket',BASE/'wolf-b.sock','fang','close',matches[0]['id']],label)
    time.sleep(1)
    response=control('wolf-b','fang.list',{},label+'-after')
    if any(f['local']=='127.0.0.1:'+str(port) for f in response['result']):raise RuntimeError('Fang did not close')

def remap_addr(value):
    head,sep,tail=value.rpartition(':')
    return head+sep+str(REMAP.get(int(tail),int(tail))) if sep and tail.isdigit() else value

try:
    if (BASE/'wolf-a').exists() or (BASE/'results.json').exists():raise RuntimeError('Run directory already used; choose a fresh directory')
    for key,port in PORTS.items():
        kind=socket.SOCK_DGRAM if 'quic' in key and key!='quic' else socket.SOCK_STREAM
        with socket.socket(socket.AF_INET,kind) as s:s.bind(('127.0.0.1',port))
    for wolf in ['wolf-a','wolf-b']:
        dest=BASE/wolf;dest.mkdir(mode=0o700)
        src=pathlib.Path.home()/'.config'/wolf
        for name in ['pelt.json','pack.json','fangs.json','active_fangs.json']:
            p=src/name
            if p.exists():shutil.copy2(p,dest/name)
        pack=json.loads((dest/'pack.json').read_text())
        for p in pack:p['address']=remap_addr(p['address'])
        write_json(dest/'pack.json',pack)
        if (dest/'fangs.json').exists():
            profiles=json.loads((dest/'fangs.json').read_text())
            for p in profiles:
                p['local']=remap_addr(p['local']);p['remote']=remap_addr(p['remote'])
            write_json(dest/'fangs.json',profiles)
    for name in ['target','large-target']:(BASE/name).mkdir(mode=0o700)
    (BASE/'target'/'baseline.txt').write_bytes(MARKER)
    block=b''.join(hashlib.sha256(i.to_bytes(4,'big')).digest() for i in range(32768))
    source=BASE/'large-target'/'large.bin'
    with source.open('wb') as f:
        for _ in range(50):f.write(block)
    source_hash=hashlib.sha256(source.read_bytes()).hexdigest()
    fixture={wolf:{name:json.loads((BASE/wolf/name).read_text()) for name in ['pack.json','fangs.json','active_fangs.json'] if (BASE/wolf/name).exists()} for wolf in ['wolf-a','wolf-b']}
    fixture['identities']={wolf:{k:v for k,v in json.loads((BASE/wolf/'pelt.json').read_text()).items() if k!='secret_key_b64'} for wolf in ['wolf-a','wolf-b']}
    fixture['ports']=PORTS;fixture['source_sha256']=source_hash;fixture['source_bytes']=source.stat().st_size
    write_json(BASE/'fixtures-public.json',fixture)
    for name,port in [('target',18080),('large-target',18081)]:
        start_process(['python3','-m','http.server',str(port),'--bind','127.0.0.1','--directory',BASE/name],name)
    a=start_wolf('wolf-a',1);b=start_wolf('wolf-b',1)
    for wolf,p in [('wolf-a',a),('wolf-b',b)]:
        exe=pathlib.Path('/proc')/str(p.pid)/'exe'
        record(wolf+' checkout provenance',os.readlink(exe)==str(BIN/'werewolfd'),{'pid':p.pid,'exe':os.readlink(exe),'sha256':hashlib.sha256(exe.read_bytes()).hexdigest()})
    command([BIN/'werewolfctl','--socket',BASE/'wolf-b.sock','fang','list'],'checkout-ctl-fangs')
    urls={key:'http://127.0.0.1:'+str(PORTS[key])+'/baseline.txt' for key in ['quic','encrypted','plain']}
    for key in urls:
        ok,detail=get_marker(urls[key],key+'-traffic')
        record(key+' application traffic',ok,detail)
    env=dict(os.environ,NO_PROXY='*',no_proxy='*',WEREWOLF_TARGET_URL='http://127.0.0.1:18080/baseline.txt',WEREWOLF_QUIC_URL=urls['quic'],WEREWOLF_TCP_ENC_URL=urls['encrypted'],WEREWOLF_TCP_URL=urls['plain'])
    write_json(BASE/'selector-environment.json',{k:env[k] for k in ['NO_PROXY','no_proxy','WEREWOLF_TARGET_URL','WEREWOLF_QUIC_URL','WEREWOLF_TCP_ENC_URL','WEREWOLF_TCP_URL']})
    choice('quic','secure-initial-quic',env)
    close_port(PORTS['quic'],'close-quic')
    choice('tcp-encrypted-v2','secure-fallback-encrypted',env)
    close_port(PORTS['encrypted'],'close-encrypted')
    choice('tcp-plain','secure-fallback-plain',env)
    for profile in ['home-web-tcp-encrypted-v2','home-web-quic']:
        command([BIN/'werewolfctl','--socket',BASE/'wolf-b.sock','fang','open-profile',profile],'restore-'+profile)
    time.sleep(2)
    choice('quic','secure-restored-quic',env)
    dest=BASE/'download.bin'
    rc,out,err=command(['curl','--noproxy','*','--fail','--show-error','--max-time','180','--output',dest,'http://127.0.0.1:19032/large.bin'],'encrypted-50mib',timeout=185)
    size=dest.stat().st_size if dest.exists() else 0
    digest=hashlib.sha256(dest.read_bytes()).hexdigest() if dest.exists() else None
    record('encrypted TCP 50 MiB SHA256',rc==0 and size==52428800 and digest==source_hash,{'curl_exit':rc,'source_bytes':source.stat().st_size,'received_bytes':size,'source_sha256':source_hash,'received_sha256':digest})
    before=control('wolf-b','fang.list',{},'before-restart')['result']
    active_before=json.loads((BASE/'wolf-b'/'active_fangs.json').read_text())
    stop_process(b);stop_process(a)
    a=start_wolf('wolf-a',2);b=start_wolf('wolf-b',2)
    after=control('wolf-b','fang.list',{},'after-restart')['result']
    active_after=json.loads((BASE/'wolf-b'/'active_fangs.json').read_text())
    expected_ports={19020,19021,19022,19032}
    restored_ports={int(f['local'].rsplit(':',1)[1]) for f in after}
    checks={}
    for key in urls:
        ok,detail=get_marker(urls[key],'restart-'+key+'-traffic');checks[key]={'ok':ok,**detail}
    rc,out,err=command(['curl','--noproxy','*','--fail','--silent','--show-error','--head','--max-time','12','http://127.0.0.1:19032/large.bin'],'restart-large-head',timeout=15)
    checks['large']={'ok':rc==0,'curl_exit':rc}
    before_ids={f['id'] for f in before};after_ids={f['id'] for f in after}
    record('daemon restart and profile restoration',restored_ports==expected_ports and active_before==active_after and before_ids.isdisjoint(after_ids) and all(x['ok'] for x in checks.values()),{'before_ids':sorted(before_ids),'after_ids':sorted(after_ids),'active_before':active_before,'active_after':active_after,'restored_ports':sorted(restored_ports),'application_checks':checks})
    choice('quic','secure-after-restart',env)
except Exception as e:
    record('runner completion',False,{'exception':repr(e)})
finally:
    for p in reversed(processes):stop_process(p)
    for h in handles:h.close()
    write_json(BASE/'process-cleanup.json',[{'pid':p.pid,'exit':p.returncode} for p in processes])
    print('Evidence:',BASE,flush=True)
sys.exit(0 if results and all(r['status']=='PASS' for r in results) else 1)
```
