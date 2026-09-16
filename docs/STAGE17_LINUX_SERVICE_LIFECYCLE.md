# Stage 17: Linux service and daemon lifecycle

## Scope and platform

Stage 17 certifies `werewolfd` as a foreground Linux x86_64 service. It adds
process ownership, signal handling, lifecycle tests, and a systemd unit. It
does not change the TCP or QUIC wire protocols, cryptography, authority model,
Pelt format, or the Stage 15B persistent-state format. Windows, containers,
live configuration reload, and traditional double-fork daemonization remain
out of scope.

## Baseline characterization

Before this stage, startup validated the Den and started the control socket,
then detached TCP and QUIC listener tasks. A listener bind error was only
logged by a detached task. There was no application signal handler: a measured
foreground `SIGTERM` exited with status 143 and left the Unix socket pathname
behind for the existing stale-socket recovery on next start. The second live
daemon failed the Den `flock`, but with the generic process error status 1.

The pre-existing security primitives were sound and are retained:

- `.den.lock` is a mode-0600 regular file held with nonblocking exclusive
  `flock`; process death releases it, while its inode is never unlinked.
- The control socket takes its own instance lock, checks a stale socket by
  connecting, and unlinks only an inode that refused the connection while that
  lock is held.
- A missing Pelt leaves the control plane available and makes secure listener
  tasks wait for the Stage 12D4 in-process identity publication.
- Missing or malformed Silver is locked; protected-state manifest failures
  fail before startup publication.

Default POSIX actions previously applied to SIGINT/SIGHUP/SIGKILL. SIGKILL
still cannot run cleanup. Listener bind failures, malformed Den state, locked
Silver, missing Pelt, and a control-socket collision were not modeled as
explicit lifecycle states or stable exit classes.

## Service states and startup ordering

The daemon logs the following local lifecycle states:

| State | Meaning |
|---|---|
| `STARTING` | argument processing and process startup began |
| `VALIDATING` | Den lock and security-state validation are in progress |
| `LOCKED` | Silver is active; control remains available and forwarding is fenced |
| `READY` | Pelt/TLS identity exists, control is bound, both secure listeners bound, and profile restoration completed |
| `STOPPING` | shutdown has fenced authority and is cancelling owned work |
| `FAILED` | startup or supervisor failure is returning a classified local exit code |

With no Pelt, the daemon deliberately does not log `READY`; it logs
`WAITING_FOR_PELT` after the control socket is available. The two secure
listeners remain unbound until a successful transactional `pelt.init` publishes
the in-memory `RuntimeTlsIdentity`. This preserves the certified Stage 12D4
ordering.

The actual startup sequence is:

1. parse arguments and establish local process prerequisites;
2. validate and open the Den, then acquire `.den.lock`;
3. load and validate the Stage 15B protected generation before publication;
4. load Pelt and derive the process-local TLS identity;
5. bind the credential-checked local control socket;
6. restore persisted Fang profiles under the existing policy;
7. start owned TCP and QUIC listener supervisors, and, when Pelt already
   exists, wait for both actual bind confirmations;
8. report `LOCKED` or `READY`.

An initial Pelt-ready TCP or QUIC bind failure now terminates startup with code
69 and closes the control socket. No forwarding listener is reported ready
before it has bound. A Stage 15B manifest mismatch returns code 65 before the
control socket or either forwarding listener is created.

## Shutdown and signals

SIGTERM and SIGINT share one bounded shutdown path:

1. log `STOPPING`;
2. lock the in-memory Stage 11C authority, cancelling establishment and active
   leases before any new application submission can pass its gate;
3. abort registered local Fang/profile tasks and their children;
4. notify the control supervisor to stop accepts, abort bounded client tasks,
   and remove only its owned socket inode;
5. abort owned TCP and QUIC listener supervisors, which in turn drop their
   owned connection and stream `JoinSet`s;
6. wait up to five seconds using Tokio monotonic time, then abort any remaining
   owned control task.

Systemd grants ten seconds with `TimeoutStopSec=10s`, leaving headroom beyond
the daemon's five-second application deadline. Active transfers may be
intentionally terminated by shutdown; this does not alter the normal
uninterrupted 50 MiB integrity contract.

SIGHUP is an explicit logged no-op. There is no live reload: local state changes
continue to use the transactional `werewolfctl` API. SIGKILL has no graceful
cleanup claim. Restart safety after SIGKILL comes from durable state, OS lock
release, Stage 15B load validation, and refused-connection stale-socket
recovery.

Local exit classes are stable for service policy:

| Code | Class | Examples |
|---:|---|---|
| 0 | success | SIGTERM/SIGINT orderly completion |
| 64 | configuration | invalid CLI or unavailable implicit runtime socket path |
| 65 | security state | unsafe Den, malformed Pelt, or invalid manifest generation |
| 66 | already running | another daemon holds the same Den lock |
| 69 | listener | required TCP/QUIC bind failure when Pelt is available |
| 70 | runtime | unexpected control or listener supervisor exit |

The systemd unit prevents automatic restart for 64, 65, 66, and 69. This avoids
a rapid restart loop for a persistent security/configuration defect.

## Crash, lock, and socket recovery

The Linux harness starts a Pelt-ready disposable Den, then performs 50 clean
start/status/SIGTERM cycles and 10 start/ready/SIGKILL cycles. Every clean stop
exited 0 and removed its control socket. Every subsequent SIGKILL restart
reacquired the released Den `flock`, replaced only the refused stale socket,
and reached local authenticated control and TCP listener readiness. A second
live daemon consistently exited 66 without disturbing the first. A deliberately
occupied TCP port caused code 69 and did not leave a control socket active.

The harness also corrupts a copied Stage 15B selector. The process exits 65 and
never creates the requested control socket. This confirms lifecycle handling
does not weaken manifest fail-closed startup.

## Network and administrative behavior

The system service orders after `network.target`, not `network-online.target`.
It binds local configured addresses and validates the Den without requiring an
Internet route at boot. Loss of external network, resolver availability, or a
target causes only the bounded establishment attempt to fail; Stage 16's
timeouts and permits release it. A later connection can retry after network,
DNS, or target recovery without restarting the daemon.

Silver remains a valid `LOCKED` service state: control stays available, the
authority is fenced, and forwarding cannot activate until the existing durable
Silver reset path succeeds. Missing Pelt similarly retains administrative
availability while secure listeners wait. An ordinary Fang failure does not
disable local administration. Once `STOPPING` begins, the control supervisor
stops accepting new requests rather than leaving them to hang.

## Linux filesystem and account layout

The provided unit uses an explicit system-service layout:

| Purpose | Path | Owner/mode |
|---|---|---|
| binary | `/usr/local/bin/werewolfd` | root-owned installation artifact, executable |
| persistent Den | `/var/lib/werewolfproxy` | `werewolf:werewolf`, `0700` |
| runtime control directory | `/run/werewolfproxy` | `werewolf:werewolf`, `0700` |
| control socket | `/run/werewolfproxy/control.sock` | `werewolf:werewolf`, `0600` |
| configuration policy | operator-provisioned Den documents | Den-validated, normally `0600` |

`StateDirectory=werewolfproxy` and `RuntimeDirectory=werewolfproxy` create the
two service directories; `UMask=0077` is compatible with the Stage 11B mode
checks. The unit passes both `--home /var/lib/werewolfproxy` and
`--socket /run/werewolfproxy/control.sock` explicitly. It does not relocate an
existing user Den. Direct foreground/user-Den operation remains unchanged and
continues to use either `--socket` or the existing XDG runtime default.

The unit uses `User=werewolf` and `Group=werewolf`. No Linux capability is
needed for the default high TCP/UDP ports, DNS, outbound target connects, Den
access, or the Unix control socket, so `CapabilityBoundingSet=` is empty. A
deployment that deliberately chooses a port below 1024 must use a separately
reviewed provisioning design; Stage 17 does not grant `CAP_NET_BIND_SERVICE`.

Create the account and install a test deployment manually:

```sh
sudo useradd --system --home-dir /var/lib/werewolfproxy \
  --shell /usr/sbin/nologin werewolf
sudo install -d -o werewolf -g werewolf -m 0700 /var/lib/werewolfproxy
sudo install -m 0644 packaging/systemd/werewolfd.service \
  /etc/systemd/system/werewolfd.service
sudo install -m 0755 target/release/werewolfd /usr/local/bin/werewolfd
sudo systemctl daemon-reload
sudo systemctl enable --now werewolfd.service
```

Initial filesystem provisioning needs administrative privileges. The daemon
itself does not: a real transient systemd service ran as `nobody:nogroup` with
the full sandbox, service-managed state/runtime directories, local `pelt.init`,
and SIGTERM completion. The fixed `werewolf` account was not pre-created on the
test host, so `nobody` was the equivalent unprivileged service-account test.

## Systemd unit and hardening

`packaging/systemd/werewolfd.service` is `Type=simple`: systemd considers the
process active after it is spawned, while the daemon's `READY` log is the exact
application readiness boundary. No `sd_notify` dependency or watchdog was
added. `Restart=on-failure`, `RestartSec=5s`, and the 60-second/5-burst start
limit avoid restart storms. `LimitNOFILE=4096` meets the Stage 16 operational
model for a TCP-heavy 1,024-session deployment. `TasksMax` and `MemoryMax` are
left to the host because no measured value can be safely fixed here.

Enabled directives are:

- `NoNewPrivileges=yes`, empty capability bounding set, `PrivateTmp=yes`;
- `ProtectSystem=strict`, `ProtectHome=yes`, and explicit Den/runtime writable
  paths;
- `ProtectKernelTunables=yes`, `ProtectKernelModules=yes`,
  `ProtectControlGroups=yes`, `RestrictSUIDSGID=yes`, `LockPersonality=yes`,
  `RestrictRealtime=yes`, `MemoryDenyWriteExecute=yes`, and
  `SystemCallArchitectures=native`.

`RestrictAddressFamilies` and a system-call allowlist are deliberately deferred:
resolver and deployment variation make an untested allowlist brittle. The unit
does not use `DynamicUser=yes`; systemd's DynamicUser state-directory indirection
uses a path form rejected by the Den's no-symlink path validation. A fixed
dedicated account provides a compatible service identity.

The exact unit passed `systemd-analyze verify` after a temporary executable was
placed at its documented `/usr/local/bin/werewolfd` path; that test file was
removed immediately. A real systemd transient test executed the actual built
daemon and control binary as `nobody:nogroup` with the complete enabled
hardening set, `StateDirectory`, `RuntimeDirectory`, no capabilities, Pelt
initialization, an asserted post-Pelt TCP bind, local control, and SIGTERM. It
completed successfully. Test-only generated state and copied binaries were
deleted immediately afterward.

## Task, resource, and security boundaries

The main lifecycle owner now owns the control supervisor and both secure
transport supervisors. The control supervisor owns bounded client tasks in a
`JoinSet`; TCP owns connection tasks, QUIC owns connection/stream tasks, and
the Fang registry owns outbound profile tasks. Shutdown aborts each owner rather
than leaving a new top-level detached task. No attacker-controlled operation is
introduced under a global lock across network I/O.

Stage 16 limits remain unchanged: 256 global handshakes, 16 per authenticated
peer, 128 QUIC contexts, 64 incoming Quinn objects, 64 bidirectional streams
per QUIC connection, 64 KiB stream and 512 KiB connection receive windows, 64
global and 8-per-peer target work items, plus the existing Stage 11C authority
limits of 1,024 global sessions, 64 per peer, and 64 QUIC streams per
connection. The new service `LimitNOFILE` complements those ceilings; it does
not replace them.

The stage retains all prior security ordering: Stage 15B protected state before
listeners, Stage 12 identity before secure bind, Stage 11C authority fencing
before shutdown cancellation, and Stage 13B one contiguous outer TLS write per
unchanged Stage 10 frame. No TCP, QUIC, TLS, Stage 10, or persistence bytes are
added or changed.

## Tests and artifact audit

`tests/stage17_lifecycle/lifecycle.py` uses only private disposable Dens and
generated test identities. It contains no packet captures, payloads, TLS keys,
traffic secrets, exporter output, or saved Pelt material. The final Stage17
secret-material audit is therefore **PASS**.

Focused lifecycle coverage includes normal control-socket shutdown ownership,
the process harness cases above, static systemd verification, and actual
transient systemd execution. The final full gates passed with 182 Rust tests
(22 `werewolf-core` unit, 12 `werewolf-core` integration, 148 `werewolfd`), 64
Python integration tests, and 12/12 acceptance checks. The only new Rust test
checks orderly control-socket shutdown ownership; the process matrix is kept as
an explicit Linux-only harness because it needs real signals, filesystem locks,
and a secure private Den parent.

No previously certified test was deleted, renamed, ignored, cfg-disabled, or
filtered from normal gates. Stage17 adds the lifecycle test and harness only.

## Claims and nonclaims

Stage 17 provides deterministic validation before forwarding, bounded
SIGTERM/SIGINT shutdown, stale runtime recovery after unclean death, safe
single-Den exclusion, unprivileged system-service operation after provisioning,
and a tested systemd hardening baseline.

It does not claim that SIGKILL is graceful, that distributed resource attacks
are impossible, that an unavailable network/target preserves a current session,
that systemd's `Type=simple` is an application readiness signal, or that
container/Windows deployments are certified.
