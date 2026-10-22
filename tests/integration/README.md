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
them, the harness writes existing-format Pack fixtures: A trusts `wolf-b` and B
trusts `wolf-a`, one entry per identity. Pack validation rejects a second entry
with the same key, so each daemon's TCP and QUIC listeners share one numeric port
and encrypted TCP reuses the `quic://` address with a `tcp://` scheme.
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

## Named-service gate (Stage23)

```bash
python3 -B tests/integration/service_gate.py [--keep-temp]
```

`service_gate.py` reuses the same `Lab` isolation, build, cleanup, source-hash
and JSON-result conventions. Unlike the acceptance runner it drives the checkout
`werewolfctl` with `--json`, because the CLI is the interface under test. It
never uses `scripts/service-*-gate.sh`, which start the historical multiwolf.

Wolf A defines services that resolve to targets behind wolf B; both receivers
start deny-by-default. The SSH target is a fixture that only exchanges SSH
identification strings, so no `sshd` is needed.

| Assertion | Required evidence |
|---|---|
| Registry CLI round trip | add/list/show match; duplicate add exits 70 |
| Fail closed | Service with a peer outside the Pack and an unknown name exit 70; no Fang opened |
| No authority from the registry | Fangs opened by name deliver no target bytes before B's `target allow` |
| Named HTTP | Exact marker over QUIC and encrypted TCP after the grant |
| Named TCP | 8 MiB full-duplex echo with SHA-256 match over QUIC and encrypted TCP |
| Named SSH | Exact `SSH-2.0-` banner through the Fang |
| Persistence | Service list identical after daemon restart |
| Silver | `silver on` closes Fangs; `service connect` then exits 70 and opens nothing |

## Discovery gate (Stage24)

```bash
python3 -B tests/integration/discovery_gate.py [--keep-temp]
```

`discovery_gate.py` extends the named-service gate to four wolves in a chain:
A trusts B, B trusts A and C, C trusts B and D (and later A), D trusts C. All
receivers start deny-by-default.

| Assertion | Required evidence |
|---|---|
| Advertise | C's signed record lists exactly its two addresses; a hostname exits 70 |
| Refresh | B pings its Pack: C answers with one record, A (advertising nothing) with none |
| Find | A finds C's record through B over QUIC (by key) and encrypted TCP (by node ID); both identical |
| Multi-hop | A's lookup for D asks B (hop 1) then C (hop 2); C rejects until it trusts A, then returns D over QUIC and encrypted TCP; A's Pack unchanged |
| No trust from discovery | A's Pack still lists only B; `service connect` to C exits 70 and opens nothing |
| Persistence | Cache and advertised addresses identical after daemon restart |
| Explicit trust | After `pack add` on A and C and C's `target allow`, 1 MiB echo through the discovered address matches |
| Pack-only relay | Once B removes C from its Pack, B returns no records for C |
| Silver | `discover find` and `discover refresh` exit 70 |

```bash
python3 -B tests/integration/service_discovery_gate.py [--keep-temp]
```

`service_discovery_gate.py` (Stage25) runs three wolves: A publishes, B and C
are both in A's Pack and both trust A. All receivers start deny-by-default.

| Assertion | Required evidence |
|---|---|
| Publishing grants nothing | A publishes `echo` and `web`; a hostname target and a duplicate exit 70; `service published` shows no visibility; B and C see nothing |
| Visibility follows grants | After A grants B the echo target, B sees only `echo` over QUIC and encrypted TCP; C sees nothing |
| Deliberate import | Discovery leaves B's registry empty; `service add echo --from wolf-a` copies the entry; `web` (not granted) and C's import exit 70 |
| Normal Fang path | `service connect echo` on B carries 1 MiB echo exactly |
| Revocation | After A removes the grant, B sees nothing and a new import exits 70 |
| Persistence | Publications identical after restart; `unpublish` removes |
| Silver | On B, `discover services` and `service add --from` exit 70 |

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
