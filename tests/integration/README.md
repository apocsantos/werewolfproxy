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

```bash
python3 -B tests/integration/routed_fang_gate.py [--keep-temp]
```

`routed_fang_gate.py` (Stage26) runs three wolves: A reaches C's echo target
through relay B with the route `wolf-c via wolf-b`. A knows C by its TCP
listener; B and C know A; B and C do not know each other. All receivers start
deny-by-default.

| Assertion | Required evidence |
|---|---|
| Both grants needed | With only C's grant (B does not grant A C's listener) and with only B's grant (C does not grant A the echo target), a session through the routed Fang is reset with no bytes delivered |
| QUIC hop | `fang active` shows `tcp via wolf-b (quic)`; 64 MiB full-duplex echo exact |
| End to end at both hops | While one session is open, B and C each report one active authority session |
| Discovery through the relay | C publishes `echo-c`; A's `discover services --peer wolf-c` returns it, `discover refresh` answers `ok` for B (direct) and C (through B), and `service add --from wolf-c` imports it |
| TCP hop | With `--hop tcp`, `tcp via wolf-b (tcp)`; 64 MiB full-duplex echo exact |
| No plain TCP | `service connect --transport tcp-plain` to the routed peer exits 70 and leaves no Fang |
| Silver on the relay | A live routed session is reset when B enters Silver; after `silver off` a new 1 MiB session is exact |
| Target finishes first | A target that answers and half-closes while the client still sends: the client gets the answer then EOF, and its send direction ends within 10 s (QUIC and TCP hop) |
| Relay address change | `pack set-address wolf-b` on A closes the routed Fang (1 closed) and resets its live session |
| Route removal | After `route remove`, the next Fang to C is direct (no `via`) |

```bash
python3 -B tests/integration/provider_gate.py [--keep-temp]
```

`provider_gate.py` (Stage27) runs four wolves: A's service `web` lists
providers B, C and D in that order. Each provider's target first sends a tag
naming it and counts its connections.

| Assertion | Required evidence |
|---|---|
| Refusal skipped | With no grant on B, C serves and B's target sees no connection |
| First acceptor serves | Once B grants, B serves and C and D see nothing more |
| No retry after establishment | B's target resets mid-session: the client sees the reset and no other provider is contacted |
| Unreachable skipped in time | B's Pack address moved to a silent port closes the Fang; reopened, C serves after B's 5 s handshake deadline, under 15 s |
| All failing | With C and D ungranted too, the connection is reset within the window |
| Pack change | Removing D from A's Pack closes the Fang; reopening fails while D is listed |
| No plain TCP | `--transport tcp-plain` exits 70 |

```bash
python3 -B tests/integration/governance_gate.py [--keep-temp]
```

`governance_gate.py` (Stage28) generates ten governance keys with
`werewolfctl governance keygen` and produces every statement with the
offline draft, sign and combine commands (seven holders each). Wolf A trusts
B and C; B serves an echo target to A.

| Assertion | Required evidence |
|---|---|
| Pinning and manifest | C without a manifest exits 70; genesis without or with a wrong `--expect-hash` exits 70; with it, all three apply, status shows the ten keys and threshold 7, and A's manifest is schema 2 |
| Revocation | A statement with six signatures exits 70; revoking B removes `wolf-b` from A's Pack, closes A's Fang to B, re-adding B exits 70, and re-applying reports `already_applied` |
| Restart | After restart A's head is sequence 1 with B revoked, and re-adding B still exits 70 |
| Gap and rotation | On B (head 0) a sequence-2 statement exits 70; a rotation of key 9 applies and is in force |
| Fork | On A the rotation conflicts with the applied revocation: exit 70, `forked` set, every later statement exits 70, and the flag survives restart |
| Admission | On B an admission applies and only updates the roster (not activated, B admitted) |

```bash
python3 -B tests/integration/activation_gate.py [--keep-temp]
```

`activation_gate.py` (Stage29) runs three wolves on one governance network: A
and B are admitted, X at first is not. A trusts B and X, and X serves an
echo target to A.

| Assertion | Required evidence |
|---|---|
| Before activation | Every node is admitted; A reaches the echo target through X |
| Activation | All three heads advance to sequence 2 with the same hash; A removes `wolf-x` and closes its Fang; X reports `admitted: false`, its inbound authority is locked, its Fang open and discovery exit 70, and A's `pack add` of X exits 70 |
| Fence holds | `silver on` then `silver off` on X leaves it locked; after a restart X is still unadmitted and locked |
| Admitted later | An admission of X lifts the fence: X unlocked and admitted at sequence 3, its discovery works, and A re-adds X and reaches the echo target through it again |
| Permanent revocation | Revoking X fences it again and removes it on A; an admission of X exits 70 on every node with no head moving, and A's `pack add` of X exits 70 |

```bash
python3 -B tests/integration/sync_gate.py [--keep-temp]
```

`sync_gate.py` (Stage29 step 3) runs three wolves on one governance network.
A and B receive statements as files; C only runs `governance sync`. C trusts
A, B and an unreachable "ghost" member; A and B trust C.

| Assertion | Required evidence |
|---|---|
| Catch-up | After five admissions on A and B, one sync applies sequences 1–5 on C, each once, over two rounds; a second sync applies nothing |
| Artefact replaced (G3) | A garbage `governance_00000001.wwgov` in C's Den beyond its head is replaced by the real statement on apply |
| Tampered entry never served | B's archive entry 3 replaced by another valid statement: B offers `[2, 1]` (stops before 3), A offers `[4, 1]`; the ghost is `unreachable` |
| Revocation through a peer | A and B revoke the ghost; C's sync removes `ghost` from its Pack and re-adding it exits 70 |
| Conflict (addition D) | A and B apply different valid admissions as sequence 7: C's sync applies nothing, stops with `GOVERNANCE_FORK`, C is forked at 6, and a further sync or apply exits 70 |

```bash
python3 -B tests/integration/death_gate.py [--keep-temp]
```

`death_gate.py` (Stage30 step 2) builds werewolfd with the `lab-faults`
feature into `target/stage30-lab-faults` (no other gate or release build
uses it). It runs disposable daemons only, with core dumps disabled for
them.

| Assertion | Required evidence |
|---|---|
| Death from a file | B and C apply the death: reply `lifecycle: dead`, `destruction: complete`; exit 67; each Den is exactly `.den.lock` (0 bytes) and `tombstone.json` (kind `death`, terminal sequence and hash of the statement); no pre-death value (Pelt strings and keys, peer names, fingerprints, addresses, Pack and governance keys, in raw, hex and base64) remains in either Den; A's echo through B fails and A's sync reports B and C unreachable |
| Offline node | D, stopped during the death, restarts normally; its sync applies nothing (no holder of the death is alive); the file kills it as above |
| Dead Den restart | Restarting B exits 67 at once, binds no listener, leaves the Den unchanged; `den init` on it exits 70 |
| Retirement | A wrong `--confirm` exits 70; the right fingerprint retires A (`complete`, exit 67); the retirement Tombstone has only `format`, `version`, `kind`, `destruction`, `software` |
| Storage fault | An unremovable entry in the Den: retirement reports `incomplete`, exits 67, the Pelt is gone; a restart stays `incomplete`; once the entry is removable a restart completes |
| Crash convergence | An abort at each step boundary (death: after the DEAD commit, after the Tombstone, after the Pelt, after the security state, before completion; retirement: the last four) is followed by a restart that exits 67 with a complete Tombstone and a clean Den |

Not run by the gate: a full disk (needs a size-limited filesystem). The
media residue measurement needs root and is a separate script:

```bash
sudo python3 tests/integration/death_residue.py [BINARY_DIR]
```

It loop-mounts a 64 MiB ext4 image, retires a disposable daemon whose Den
is on it, unmounts, and counts occurrences of the Pelt secret and a Pack key
in the raw image. Counts are reported as a measurement of that filesystem,
never as erasure.

```bash
python3 -B tests/integration/auto_sync_gate.py [--keep-temp]
```

`auto_sync_gate.py` (Stage31 step 1) uses the same `lab-faults` build, which
lets it set exact automatic-sync delays (first run 300 ms after a node
becomes eligible, then every 1.5 s). Five disposable daemons: A, B, C and D
admitted; U not, at first. A UDP sink stands in for one Pack member of B
(`wolf-trap`, its key admitted) and counts the datagrams B sends it.

| Assertion | Required evidence |
|---|---|
| Ceremony stays manual | Admission applied by file on A only; after three intervals B, C and D are still at sequence 0, and every node reports `not_activated` with no run |
| Runs after activation | Activation applied by file everywhere: A, B, C and D each start a run on their own (`nothing_new`); U reports `unadmitted` and has no run |
| Revocation propagates | D revoked by file on A only: B and C reach sequence 3 with no command; B's automatic apply reports `revoked_members: ["wolf-d"]` and `closed_fangs: 1`, and B's Fang to D is gone; B's and C's archive entry 3 is byte-identical to the file; D's own runs are `rejected` by every former peer, it stays at sequence 2, and the file makes it `unadmitted` |
| Unadmitted node starts | U's Admission by file (with the revocation before it): U runs on its own, `nothing_new` from `wolf-a`; B and C pull the Admission |
| Switch at the responder's side | With the switch on, the sink receives datagrams; `auto-sync off`, then a restart of every daemon: B reports `switched_off`, and the sink receives none over three intervals plus a lookup budget |
| Operator first | While B's run is resolving (held by the sink), `governance sync --peer wolf-a` succeeds at once and B's run ends `cancelled` |
| Silver | Silver on C: `silver`, `ineligible`; Silver off: a new run follows |
| Fork | A and C apply different valid statements by file (C and B switched off first); B switched on: its run ends `fork`, B is forked and `ineligible`, and it does not run again |
| Applied once | B's and C's logs show each automatic apply (sequences 3 and 4) exactly once |

Every sampled run is also checked for a `rejected` status from a real
peer, which would show a responder refusing it (for example its token
bucket). DYING is covered by unit tests and the death gate.

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
