# Disposable integration lab (Stage 1)

`BASELINE.md` is the authoritative Stage 0 contract. This lab reproduces its
application checks using fresh identities and dynamically allocated loopback ports.
It does not consume Stage 0 evidence or live Werewolf configuration.

From the repository root:

```bash
python3 -B -m unittest discover -s tests/integration -p 'test_*.py'
python3 -B tests/integration/run.py
# Explicitly retain private fixtures, logs and results for investigation:
python3 -B tests/integration/run.py --keep-temp
```

Prerequisites: Linux with `/proc`, Python 3.11+, Bash, curl, Git, Cargo/Rust and
cached dependencies matching Cargo.lock. TCP/UDP loopback and Unix-socket creation
must be permitted. Stage 0 used Rust 1.96.0; the runner records actual tool versions.
No Python packages or new Cargo dependencies are required. Restricted execution
environments may require permission to run the lab with local networking enabled.

Every acceptance invocation builds:

```bash
cargo build --locked --offline --workspace --bins --target-dir <checkout>/target/stage1-lab
```

Only the absolute checkout `target/stage1-lab/debug/werewolfd` is exercised.
`werewolfctl` and other workspace binaries are built but not executed. Control
operations use newline-delimited JSON directly; CLI text is not parsed. The build
uses the invoking Rust environment/cache; runtime HOME, XDG directories, TMPDIR,
PATH and selector URL variables are explicitly isolated. Installed Werewolf
binaries are never used. Build artifacts remain under the ignored `target/` tree.

Each invocation creates a mode-0700 `/tmp/wwp-lab-*` directory (short paths also
avoid Unix socket length limits). This holds:

- `a/`, `b/`: fresh Pelt identities, Pack files, Fang profiles and active state;
- `a.sock`, `b.sock`, runtime/config directories;
- target marker, deterministic 50 MiB source and downloaded payload;
- public fixture/port mapping, executable provenance, JSON control transcript,
  process commands, selector output, results and cleanup evidence.

The harness implementation is repository-contained; disposable runtime state is
outside the source tree. By default all temporary state is removed on success,
failure or handled SIGINT/SIGTERM. `--keep-temp` retains it even after failure,
including **private test identity keys**. All created process groups are terminated
in either mode. The final stdout line is JSON with overall status and test results;
preceding lines provide progress and a concise PASS/FAIL matrix. Preserve stdout
externally if results must outlive default cleanup. Setup, assertion, timeout,
cleanup and source-preservation failures return nonzero. Readiness has bounded
polling; failed acceptance assertions are not retried.

## Provisioning and contract

Two daemons first initialize fresh identities through `pelt.init`. After stopping
them, the harness writes existing-format Pack fixtures: A trusts B, and B has
`wolf-a` (QUIC) and `wolf-a-tcp` (encrypted TCP) aliases sharing A's public identity.
The daemons restart to load Pack. No live Pelt or Pack is copied. Profile creation
and activation use control operations, allowing production code to persist state.

The four Stage 0 profile names and transport fields are preserved. Both HTTP
fixture paths share one private HTTP server; its port is chosen by binding port 0.
A separate raw TCP echo target also binds port 0. Daemon/Fang ports use held
reservations released immediately before startup. No fixed production ports or
sockets are configured. Logs include the actual mappings.

| Assertion | Required evidence |
|---|---|
| QUIC application traffic | Exact Stage 0 51-byte marker |
| TCP encrypted v2 (`tcp`) | Exact marker |
| TCP plain (`tcp-plain`) | Exact marker |
| Secure fallback | QUIC → encrypted v2 → plain JSON selections and exact GET after each |
| Restore QUIC | Reopen encrypted/QUIC profiles; QUIC selected and exact GET |
| Encrypted TCP integrity | Exactly 52,428,800 bytes; expected SHA256 below |
| Daemon restart | Four Fangs restored without manual reopen, new IDs, same active list and identities/Pack; three GETs, large-file HEAD, QUIC selection |
| Additional raw TCP check | Exact 16 KiB binary echo across each transport; transient Fangs closed before restart |

Expected SHA256:
`80d160f35ab0b90c95b2f4777c0fa0127bb2a4b5f6fff2f4a6c0f3a4c1219ea6`.

The runner reuses Stage 0's marker/payload algorithm, close/reopen failure injection,
selector invocation and restart assertions. It invokes only the unchanged
`scripts/wolf-b.sh auto --policy secure --json` branch with all four URL overrides.
No systemd, timer, watchdog, healing script, installed wrapper or historical gate
is involved. Selection checks use new connections, matching Stage 0; they do not
claim migration of established streams. Plain TCP reaches the target directly.

All acceptance assertions use JSON/control data or application bytes. There is no
unavoidable human-output parsing in this lab. Cargo/compiler diagnostics are logs,
not behavioral assertions. Source files are hashed before/after each run, including
existing dirty files and BASELINE.md. See KNOWN_ISSUES.md for unchanged production
limitations. The lab does not redesign or test a replacement authentication scheme.

## Stage 1 execution results — 2026-09-07

Executed against checkout `63691aade2aa165b16864a80272f2a55eadcee00` with its
Stage 0 dirty working tree, Rust 1.96.0 and locked offline dependencies. The lab
built its own debug binaries; installed binaries and historical results were not
used as evidence. Socket permission was required outside the execution sandbox.

| Executed check | Result |
|---|---|
| QUIC / encrypted TCP v2 / plain TCP exact application bytes | PASS (each) |
| Secure QUIC → encrypted v2 → plain selection and application traffic | PASS |
| Restoration to QUIC | PASS |
| Encrypted 50 MiB size and baseline SHA256 | PASS |
| Restart both daemons and restore all four profiles | PASS |
| Binary TCP echo across all three transports | PASS |
| Harness failure/cleanup tests | PASS, 10 tests |
| Default removal of private runtime state | PASS |
| BASELINE.md and three protected dirty-file hashes | Unchanged from Stage 0 |

No new production defect was observed. The initial sandbox-denied invocation
failed loudly and cleaned up; it is not counted as an acceptance pass. The
successful run used the commands above with local socket permission. Future runs
emit their own JSON results and provenance; this table records execution during
Stage 1 rather than guaranteeing later environments.
