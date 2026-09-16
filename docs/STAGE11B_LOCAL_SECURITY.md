# Stage 11B — Linux local authority and persistent state

Stage 11B starts at `34623e7cd098b924235a75bccbc9ee92da4ca816` on
`codex/stage11-security-boundary-audit`. It hardens local control and persistent
state without changing network handshake transcripts, cryptographic primitives,
Pack trust meaning, target grants, transport selection, or established inbound
session authority. Linux x86_64 is the supported certification platform.
Windows is not supported or certified by this stage.

## Authority and threat model

The daemon obtains connecting credentials using Tokio `UnixStream::peer_cred()`
(Linux SO_PEERCRED). Only its own effective UID is ADMIN. Credential-query errors
and every other UID are rejected before parsing or administrative response.
Root has no special application bypass; a root daemon naturally has UID 0 as its
own UID. Filesystem access alone does not authenticate a caller.

All commands use this one explicit authority class: status, den.info,
pelt.fingerprint/init, pack.list/add/set_address/remove/revoke,
fang.open/open_profile/close/list/cleanup, fang.profile.list/add/remove, and
silver.trigger/reset. There is no READ/OPERATOR role, Unix-group delegation,
remote negotiation of authority, or exactly-once control delivery promise.

Same-UID malicious processes, root, a malicious kernel/filesystem administrator,
and malicious filesystem rollback are outside the isolation claim. A dedicated
service account provides a stronger boundary than sharing an ordinary desktop
account. Remote peers do not acquire local administrative privilege from Pack
membership. Authorized local responses may contain useful public configuration;
private identity material is not returned or logged.

## Runtime directory and socket

Default daemon and CLI discovery is
`$XDG_RUNTIME_DIR/werewolf/control.sock`, with no `/tmp/werewolf.sock` fallback.
XDG_RUNTIME_DIR must itself pass private-directory validation. Explicit
`--socket` paths must place the daemon socket inside a validated private parent.
The daemon may create the missing final directory only, with mode 0700 from
creation. It does not repair existing ownership or permissions.

The per-wolf scripts use `$XDG_RUNTIME_DIR/werewolf-{a,b,c,d}/control.sock`.
Control commands require XDG_RUNTIME_DIR. Selector-only operations still work
without that variable; no selector policy or fallback order was changed.

Private leaves must be directories, effective-UID/effective-GID owned, mode
exactly 0700. Traversal starts from a root directory descriptor and opens every
component with DIRECTORY, NOFOLLOW and CLOEXEC. Parent traversal and `..` are
not accepted. Ancestors must be root-owned or owned by the effective UID/GID,
and must not be group/world writable. A root:root sticky ancestor is the limited
exception for temporary storage; it is never accepted as the private leaf.
Set-ID execution is rejected. Creation of a leaf synchronizes its parent.

Socket lifecycle:

1. Validate and retain the private directory descriptor.
2. Acquire a nonblocking exclusive lifetime lock on the private regular lock file.
3. Inspect the socket entry using descriptor-relative statat/SYMLINK_NOFOLLOW.
4. Probe existing sockets with a one-second bound. Success is active; timeout and
   errors other than connection-refused are ambiguous and preserve the entry.
5. Only a confirmed stale socket with revalidated device/inode/type/UID/GID may
   be removed with unlinkat. Regular files, links and other objects are preserved.
6. Bind through `/proc/self/fd/<retained-dir-fd>/<name>`.
7. Verify socket type/UID/GID/link count and record device/inode; chmodat to 0600;
   immediately recheck no-follow metadata, identity and exact mode.
8. Only the successfully constructed ControlListener can enter accept().

The chmodat operation uses supported Linux flags that may follow the final
component. This narrowly scoped exception relies on the enclosing owner-validated
0700 directory. Different-UID processes cannot replace its entries. The pre/post
checks detect substitution but do not claim protection against a malicious
same-UID ADMIN or root. No unsafe syscall, fchmodat2 requirement, temporary
process-global umask change, blind unlink, or insecure platform fallback exists.
Lock files are not routinely removed. Socket cleanup remains the daemon's job;
launch/service scripts no longer unlink sockets.

## Control resource limits

| Resource | Limit |
|---|---:|
| Request including LF | 16 KiB |
| JSON depth | 16 |
| Request ID | 128 UTF-8 bytes |
| Command | 64 ASCII bytes |
| Peer/profile name | 128 UTF-8 bytes |
| Address/local/remote string | 512 UTF-8 bytes |
| Concurrent authorized clients | 16 |
| Concurrent mutations | 1 |
| Pending mutations | 8 |
| Requests per connection | 32 |
| Idle/first-byte and complete-request deadline | 5 seconds |
| Connection lifetime | 60 seconds |
| Mutation admission / read operation budget | 10 seconds |
| Response including LF | 1 MiB |
| Response write deadline | 5 seconds |

Bounded buffered reads preserve pipelined bytes. Duplicate JSON fields, unknown
closed-schema fields, wrong types, excessive depth, invalid JSON/UTF-8 and
trailing non-whitespace data are rejected. Explicit args must be an object;
omission retains the existing empty-args convention. Unknown commands do not
become another command. Unauthorized clients receive no administrative payload.
Slow/oversized clients lose their connection; permits release on every normal
failure path. Response serialization itself is capped, not merely its write.

An accepted mutation runs independently of the client task and holds its
coordinator permit through publication. Client disconnect or response failure
cannot cancel an already accepted persistent mutation. Queue admission has a
finite budget; OS filesystem I/O is not promised to complete within ten seconds.
A blocked storage worker can deny availability, but cannot create unbounded
mutation work or safely be cancelled as though nothing had committed.

## Den and persistent state

The daemon validates and locks its Den before restoration/listener work. A
`.den.lock` regular file is held for daemon lifetime. All daemon mutations use
the same retained directory descriptor, not a reopened configured pathname.
Standalone core save APIs also acquire this Den lock and cannot bypass a running
daemon writer. Standalone APIs require an existing secure parent directory.

`pelt.json`, `pack.json`, `fangs.json`, `active_fangs.json`, `target_policy.json`,
standalone `peers.json`, lock files and temporary state files require effective
UID/GID ownership and exact mode 0600. Persistent files must be regular with
link count one. Symlinks, hard links, directories, FIFOs, sockets and device
metadata fail closed. File opens use NOFOLLOW/CLOEXEC and NONBLOCK where special
files could otherwise stall. Checks validate the opened object and compare
identity with no-follow entry metadata. The destination is never opened for
truncate-in-place writing.

Security documents and writes are bounded to 1 MiB; daemon identity loading is
bounded to 4 KiB. Startup validates all named security models before restoring
Fangs or accepting control/network work. Missing identity/Pack/profiles/active
intent is uninitialized/empty state. Invalid identity, trust, profile or active
intent blocks startup. Unknown temporary files are never loaded as named state.

Target policy preserves Stage 9: missing means deny; malformed/unknown-mode
content means invalid/deny; unsafe filesystem metadata/path or read-integrity
failure blocks startup. Rules, canonicalization, DNS authorization, explicit
legacy-allow and startup-only loading are unchanged.

Pelt validation decodes canonical 32-byte keys, derives the public key from the
private key, and verifies the stored public key and fingerprint. Its Debug output
is redacted. `pelt.init` and the standalone identity save API are initialization
only. A valid initialized daemon returns ALREADY_INITIALIZED on repetition.
Invalid/unsafe existing state is not replaced. Concurrent initial creation uses
NOREPLACE, so only one winner can publish. No online force/rotation is provided.

Pack validation caps entries at 1024 and rejects duplicate names, fingerprints
and public keys, malformed canonical fingerprints/keys and conflicting
derived fingerprint/key pairs. Existing optional-public-key representation is
preserved; absence does not invent a new authentication or trust mechanism.
Profiles and active intents are capped at 512 with unique names/references and
validated endpoint/transport syntax. A removed peer may leave an inert stored
profile: startup accepts its structurally valid configuration, while opening it
still fails the current Pack lookup. Adding a new profile requires a current
peer. Removing an active profile is rejected until its active intent is closed.

## Atomic commit and publication

The common writer:

1. Revalidates the retained parent and destination without following links.
2. Creates an OS-random, exclusive same-directory temporary file, mode 0600.
3. Verifies the opened file, writes the complete serialized candidate, flushes,
   and fsyncs the file.
4. Revalidates destination and temporary inode metadata.
5. Renames atomically; a missing destination/initial identity uses NOREPLACE.
6. Fsyncs the containing directory.
7. Only then publishes the corresponding candidate in memory.

| Outcome | Contract |
|---|---|
| NotCommitted(error) | Before rename; previous named state survives. No memory publication or success. |
| DurablyCommitted | File and directory synchronization succeeded; publish matching memory. |
| IndeterminateAfterRename(error) | Never claim rollback or success. Mark storage degraded, reject further administrative mutations, report operator intervention. |

The degraded flag is exposed in local status/den.info. It is not a claim that
all previously established traffic has stopped. Controlled restart/operator
inspection is required. A storage-worker panic is conservatively indeterminate.
The standalone io::Result adapter reports an explicit IndeterminateAfterRename
error; callers must not treat it as a successful rollback.

Temporary cleanup occurs only for the writer's own still-matching inode. A
collision or replacement is preserved. Incomplete temporary files left by a
crash are ignored at startup. Atomicity does not prevent malicious rollback or
prove durability on a filesystem/storage stack that lies about fsync.

Persistent model changes follow candidate -> durable commit -> memory publish.
There is no mutate-first/rollback-later pattern for those models. No synchronous
file write occurs while holding the shared daemon-state mutex. The serialized
mutation worker retains ownership after client disconnect.

Profile activation prepares/binds resources without application accept, commits
activation intent, publishes the registry, then releases a private watch gate.
The kernel may queue TCP connections to a prepared listener, but application
accept/forwarding cannot start until release. Abandonment fails the gate and
failed preparation is aborted. Close commits intent removal, publishes removal,
then cancels runtime resources. Failure after durable intent that prevents safe
publication marks storage degraded. No established inbound-session authority or
Silver redesign is part of this work.

## Security test evidence and fault matrix

Before the documentation commit, full validation passed with 88 Rust tests,
64 Python tests and 12 isolated acceptance checks. Both historical tests
`actual_daemon_restart_crash_and_pelt_regeneration` and
`legacy_interoperability_matrix` executed using real Git historical fixtures.
The initialization test now explicitly expects rejection and verifies unchanged
identity bytes, fingerprint/public key, valid reloading and a successful frozen
identity ACK. A concurrent initialization test requires exactly one winner.

| Matrix | Evidence |
|---|---|
| Runtime modes | Exact 0700 accepted; 0750/0755/0770/0775/0777 rejected; socket final mode 0600 |
| Socket lifecycle | Active preserved, stale cleanup, competing startup/lock exclusion, regular/symlink/FIFO collision rejection |
| Socket metadata | Foreign UID/GID and device type injected; post-chmod inode substitution injected and rejected |
| Credentials | Real same-UID socket; different UID/root/query-failure decisions injected, not a real second-UID process |
| Control resources | Fragmented/pipelined, no LF, oversized, malformed/duplicate/deep JSON, slow client/reader, quota/lifetime/request-count/queue release |
| Persistent objects | Symlink victims inside isolated fixtures, hard links, FIFO/socket/directory, temp collision, substitution before rename |
| Model integrity | Pelt mismatch, Pack duplicates/conflicts, unsafe/malformed target policy, inert removed-peer profile, incomplete temp ignored |
| Transactions | Failed Pack write leaves memory unchanged; client cancellation cannot cancel publication; active open/close write failure preserves registry/intent |
| Pinned Den | Path moved/replaced; mutation stays on the retained original directory |
| Umask | Complete Rust suite passed under separate-process umasks 0000, 0002 and 0077 |

| Injected failure point | Named state / reported outcome |
|---|---|
| Before temp create | Old / NotCommitted |
| After temp create | Old / NotCommitted |
| Mid-write | Old / NotCommitted |
| Before file fsync | Old / NotCommitted |
| Before rename | Old / NotCommitted |
| After rename, before directory fsync | Complete new / IndeterminateAfterRename |
| After directory fsync, before publication return | Complete new / conservatively IndeterminateAfterRename |

These are deterministic injected failures, not physical power-loss tests. The
mode/device/foreign-UID cases that cannot be performed as this ordinary user use
metadata/credential injection, without unsafe Rust or destructive external victims.

Acceptance retains QUIC, encrypted TCP and explicitly requested plain payloads;
STRICT exhaustion; COMPATIBILITY without/with explicit plaintext authorization;
LEGACY fallback; unknown-policy rejection; restoration to QUIC; encrypted 50 MiB
SHA-256; binary traffic through all three transports; and restart restoration.
Target-authorization, replay, receiver-pinning, parser/admission and exporter
properties remain in the Rust regression suite.

## Dependencies and preserved boundaries

Only the reviewed direct Linux core dependency was added:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
rustix = { version = "=1.1.4", features = ["fs", "process"] }
```

Cargo.lock adds only `"rustix"` to werewolf-core's dependency list; no package
node/version/checksum or existing edge changes. Its SHA-256 is
`95989811283b1328a723ca0545595ffcd3b2bd833576162df2f9c3feaaa473b8`.
Rustix was already locked through the prior graph (including tempfile); process
supports own UID/GID queries, not peer credential extraction. Resolved rustix
features: alloc/default/fs/process/std. Linux-raw-sys 0.12.1 features:
auxvec/elf/errno/general/ioctl/no_std/prctl. No net feature is enabled.
The accepted advisory/API checkpoint found no applicable blocker; no dependency
upgrade is part of Stage 11B. Quinn 0.11.9, quinn-proto 0.11.15, quinn-udp 0.5.14
and rustls 0.23.40 remain unchanged.

Stage 10 handshake.rs, admission.rs, policy.rs and quic_lab.rs are byte-preserved
against the Stage 11A baseline. Stage 9 target_policy.rs differs only in exposing
its parser within the daemon for secure startup loading. The selector `auto`
policy block is byte-preserved. Forwarder changes are local activation waits,
not TCP/QUIC transcript changes. Core crypto primitives and session framing/KDF
are unchanged. The legacy local selector name tcp-encrypted-v2 still denotes the
TCP v3 implementation and is not wire-version negotiation.

## Deployment, migration and rollback

Production recommendation: dedicated unprivileged Unix account, private Den,
private runtime leaf, no shared-account applications. A user service is also
supported with the documented same-UID limitation; foreground use does not
require systemd. Application validation remains mandatory regardless of service
UMask. System-service administrators may use RuntimeDirectory/StateDirectory with
0700 modes, UMask=0077, NoNewPrivileges, an empty capability bounding set and
appropriate ProtectSystem/ProtectHome/PrivateTmp policies. These are deployment
recommendations, not installations performed by certification.

Before upgrading an existing deployment, stop its daemon and inspect ownership,
symlinks/hard links and contents offline. Deliberately migrate private directories
to 0700 and state files to 0600 under the intended service UID/GID. The daemon
never silently repairs them. Resolve duplicate Pack identities rather than
aliasing a key under multiple names. Update explicit socket paths and clients;
old direct-/tmp socket paths intentionally fail. Existing valid JSON persistence
formats are unchanged. Repeated pelt.init now rejects; identity replacement is
an offline future operation. Review existing target grants; absence still denies.

Rollback uses reviewed revert commits only. Stop the service before binary or
socket-location migration and keep a protected backup of valid state. Preserve
private directories and credential authorization; do not roll back to blind
unlink, permissive modes or a shared-/tmp default. No format conversion is needed.
Reverting permission/authority enforcement would reopen a known boundary and
requires explicit security review, not an automatic rollback step.

## Certification procedure and limitations

From a clean committed tree, freeze repository source for all gates:

```text
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --locked --offline --workspace
python3 -B -m unittest discover -s tests/integration -p 'test_*.py'
python3 -B tests/integration/run.py
git diff --check
```

Also run the separate-process umask matrix, shell syntax checks, exact dependency
comparison and protected-source hashes. Use disk-backed TMPDIR and
CARGO_TARGET_DIR. Historical tests require Git metadata/history. Stop on failure;
never change source while a certification sequence runs. Final certification and
full commit SHAs are reported after the clean final-state rerun.

MUST FIX IN 11B items covered here are credential authorization, safe runtime
socket lifecycle, finite control resources, no-follow/atomic persistent access,
startup semantics, serialized durable publication, initialization-only identity,
Den writer exclusion and fixture/client migration.

SHOULD FIX / DEFER: full POSIX extended ACL policy remains deferred; exact mode
checks are not a claim of full ACL enforcement. Full secret-memory zeroization
is deferred; no zeroize dependency was added. New persistence code minimizes
copies and never logs serialized secrets, but ordinary heap buffers remain.

Explicitly deferred: active inbound-session revocation, Silver established-session
authority, session lifetime/bandwidth quotas, Hide/wire opacity and handshake
metadata confidentiality, traffic morphing, TLS verification redesign (including
SkipServerVerification), Windows support, malicious root/same-UID process
isolation, filesystem rollback resistance, hardware-backed keys, distributed
identity rotation and multi-user RBAC/group delegation. Atomic writes do not
solve these issues. Network filesystems and dishonest storage durability are not
certified by this Linux local-filesystem test evidence. No Stage 11C or 12 work
is included.
