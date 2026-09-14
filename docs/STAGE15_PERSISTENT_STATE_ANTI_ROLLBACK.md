# Stage 15A — persistent-state rollback characterization

**Scope and result.** This is a study of Linux daemon startup and persistent
security state at Stage 14 HEAD `afac84608eeed42f47ef5747f179e344c24117d4`.
It adds disposable-Den characterization tests, with no production persistence,
protocol, trust, or recovery change. The primary architecture recommendation is
**EXTERNAL_MONOTONIC_ANCHOR_REQUIRED** for a genuine anti-rollback claim. Local
commitments could improve mixed-state consistency, but a rolled-back commitment
stored with the Den cannot establish freshness.

## Current persistence architecture and inventory

`werewolfd --home` defaults to `~/.config/werewolf`; `expand_home` expands `~`
using `HOME`. The effective Den must be an absolute path accepted by
`PrivateDirectory::open`. The daemon pins that directory and holds its
`.den.lock` for its lifetime. Each filename below is resolved as a single
descriptor-relative child of that pinned Den, never by following a symlink.
The lock file and control socket are operational objects, not policy revisions.
No other security-policy document is read by `load_startup_state`. The reusable
`werewolf-core::PeerStore` persistence API is not part of the daemon startup
state.

| Den child | Schema and writer | Startup reader; missing / malformed | Rollback impact |
| --- | --- | --- | --- |
| `pelt.json` | `PeltIdentity` (public key, private seed encoding, fingerprint); ADMIN `pelt.init`, create-only | `load_startup_state` checks secret/public/fingerprint consistency; absent means no Pelt and secure listeners wait; malformed aborts startup | **HIGH**: restoring an older valid identity can restore a previously trusted receiver identity. There is no supported Pelt rotation; the old/new fixture models external replacement or snapshot restore. |
| `pack.json` | array of `PeerRecord` (name, fingerprint, address, trust, optional full public key); ADMIN Pack mutations | absent means empty Pack; malformed/invalid entries abort startup | **CRITICAL**: an old valid entry can reauthorize a removed peer after restart, subject to other current gates. |
| `fangs.json` | array of named `FangProfile` (peer, local, remote, transport); ADMIN profile mutations | absent means no profiles; malformed/invalid syntax aborts startup; a removed Pack peer leaves a profile inert | **HIGH** when paired with an old Pack and active list: an obsolete listener/route can be restored. |
| `active_fangs.json` | array of active profile names; ADMIN Fang activation/close persistence | absent means none; malformed or name absent from `fangs.json` aborts startup | **HIGH** when its referenced profile and peer are also restored: startup attempts to reopen the old route. |
| `silver.json` | `{ "version": 1, "mode": "open" \| "locked" }`; ADMIN `silver.trigger/reset` | absent means **LOCKED**; malformed/unknown version or mode aborts startup | **CRITICAL**: restoring old `open` after durable `locked` makes restart start unlocked. |
| `target_policy.json` | `legacy-allow` or `deny-by-default` with exact per-peer IP/port grants; externally administered, no daemon mutation command | absent means Deny; malformed content becomes Invalid and denies authorization; unsafe filesystem object aborts startup | **CRITICAL** if an old allow rule or `legacy-allow` returns. This file has no daemon-coordinated write today. |

`PrivateDirectory` requires a private 0700 Den and owner/group checks, safe
ancestors, single regular owner-owned 0600 files, link count one, bounded reads,
and no-follow descriptor-relative access. Writers use a random exclusive 0600
temporary file, complete write, file fsync, checked rename, and directory fsync.
The daemon coordinator publishes a candidate only after `DurablyCommitted`;
`NotCommitted` leaves the candidate unpublished, and
`IndeterminateAfterRename` degrades storage rather than claiming rollback.
Silver ON fences runtime authority before its persistent write; OFF opens only
after durable reconciliation. These are access and crash-consistency properties,
not proof that a syntactically valid document is the newest one.

There is no durable global or per-file revision, state commitment, previous
hash, signed checkpoint, or monotonic anchor in the six startup documents.
Stage11C peer generations and Silver epoch exist only in runtime memory and
restart from startup state; they protect in-process authority ordering, not
restart freshness. The Den lock serializes live writers, not backup restoration
while the process is stopped.

## Threat model and distinct guarantees

| Attacker | Current boundary | Possible rollback |
| --- | --- | --- |
| A: ordinary other-UID unprivileged process without Den write access | Den owner/mode, safe ancestry, lock and local control credentials bar ordinary writes | Cannot normally replace a protected file. |
| B: process with Den write access but no daemon memory | Outside Stage11B same-UID isolation claim | Can install an older individually valid 0600 document, subject to filesystem controls. |
| C: backup/snapshot restorer | A valid old file can pass syntax and ownership checks | Can restore one file or the whole Den. |
| D: root/admin | Outside the Den access-control threat model | Can replace files and any local same-machine anchor within its control. |
| E: offline disk attacker | Outside running-process protections | Can restore an older disk image or copy with acceptable metadata. |
| F: whole-machine image rollback attacker | Controls Den, local anchors, and machine state together | Cannot be reliably distinguished from that old machine state using only data in the image. |

**Rollback detection** compares accepted state with a trusted later observation.
**Rollback prevention** refuses an older state before it becomes operational.
**State consistency** checks whether files belong to the same committed set.
**Crash consistency** accepts the correct durable outcome after interruption.
**Fork detection** identifies divergent histories given an independently retained
checkpoint. A local digest can provide consistency without freshness; a signed
old state can remain authentic but stale. File mtimes and wall clocks are
diagnostic only, not trusted monotonic anchors.

## Disposable-Den attack characterization

Five tests in `crates/werewolfd/src/persistence.rs` use generated identities,
the production `PrivateDirectory::replace` writer, and a fresh `DaemonState`
passed to the production startup reader for each simulated restart. They never
touch a user Den or retain identity bytes as artifacts. These tests describe
**current acceptance**, not a desired security contract. They do not claim to
exercise an actual network listener or a subprocess restart.

| Restore sequence after a newer durable document | Startup observation |
| --- | --- |
| old Silver `open` after `locked` | accepted; authority starts unlocked |
| old Pack with a removed full-key peer after empty Pack | accepted; peer returns to Pack |
| old exact target grant after empty grants | accepted; authorization for that endpoint returns |
| old Pelt and old Pack/profile/active Fang documents | accepted; old identity and active profile return to startup state; normal startup restoration can then attempt that profile, subject to Silver, Pack and bind checks |
| old target policy with newer locked Silver and newer Pelt | accepted as a cross-generation mixture |
| earlier valid Den documents restored together | accepted; no retained freshness evidence exists |
| active profile name absent from `fangs.json` | rejected by existing semantic validation; this is a narrow cross-file consistency check |

Mixed examples have different practical outcomes. New Silver `locked` still
blocks forwarding even if old Pack or Fang state returns. Old Pack plus new
restrictive target policy restores identity trust but not old target grants.
New Pack plus old permissive target policy may allow currently trusted peers to
use the older grants. Old Pelt plus new Pack is loadable; whether remote peers
trust that restored receiver depends on their selected full Pelt keys. A
malformed policy denies targets, whereas a *valid old allow policy* can re-enable
them. No test asserts that every mixed snapshot is accepted.

## Candidate architecture comparison

| Model | Useful guarantee | Freshness limit and crash/recovery issue |
| --- | --- | --- |
| A: Den-wide revision | With a committed manifest that binds every protected document, reject mismatched sets and stale files **relative to a trusted newer manifest**. | A revision file restored with the documents also rolls back. Per-file in-place revision writes would create legitimate mixed sets mid-commit. The manifest/pointer commit point and recovery of prior durable state need design. |
| B: BLAKE3 hash chain | Can commit ordered transitions and expose divergence if a later trusted chain head survives. | A complete older chain and state validate together. History retention, compacting and recovery are additional costs. |
| C: Pelt-signed state | Authenticates state written by the signing identity against an attacker lacking that key. | Old signed state remains valid; authenticity is not freshness. The Pelt private seed resides in the same Den, so a Den reader/writer may have the signing key too. Pelt restore or loss complicates verification and recovery. No reason to couple Pelt rotation to state revision here. |
| D: separate local anchor | If outside the restored Den and independently preserved, can detect Den-only rollback. | Same-filesystem, whole-volume, root, offline, or machine-image rollback can include it. Ordering its write against Den commit makes crash recovery and `IndeterminateAfterRename` difficult. |
| E: external monotonic anchor (paper only) | A properly provisioned non-rollbackable value can reject state older than the last anchored commit, within that anchor's threat model. | TPM NV has hardware/platform availability and write endurance; an OS protected store may share the machine's rollback domain; remote signed checkpoints add network availability and account/recovery dependencies; offline administrator checkpoints require careful manual custody. All can cause fail-closed denial of service on loss/mismatch. |

For any future canonical commitment, bind an explicit domain, schema version,
file identity, revision, and deterministic serialization of content. JSON
object key order and whitespace must not affect the committed meaning. BLAKE3
is already available, but choosing a hash is not itself a freshness solution.
No new signature, key, schema, or manifest is implemented in this stage.

### Crash-consistent local consistency proposal (possible Stage15B)

Investigate a manifest/immutable-generation design, **only for mixed-state
consistency**: build and fsync a complete candidate generation, then atomically
publish a small manifest naming the exact hashes of the six documents. Startup
validates the manifest and all referenced documents before publishing runtime
state or opening network sockets. A prior durable manifest must remain usable
after `NotCommitted`; `IndeterminateAfterRename` must remain explicitly
indeterminate and require reconciliation. The manifest swap is the proposed
logical commit point, and the recovery rule must be proven against injected
failures before implementation. In particular, `target_policy.json` is
externally administered today: the policy update workflow must be brought
under the same transaction, or the consistency claim must exclude it. Pelt is
create-only and may warrant a separate identity commitment or explicit
binding, rather than pretending it rotates with ordinary policy state.

Local revisions alone should never be labeled anti-rollback against whole-Den
restore. Even a manifest and chain stored inside the Den can be replaced by
an earlier internally consistent generation. A production Stage15B would need
its own architecture review and compatibility/recovery plan; no migration is
authorized here.

## Startup, recovery, and backup boundary

Current startup opens/locks the Den, reads and validates all six files, derives
the runtime TLS identity from Pelt, then binds the control socket and restores
profiles/listeners. It prints startup diagnostics and may create/lock the Den
before validation, but does not open a network listener or publish an
operational runtime state before `load_startup_state` succeeds. The malformed
target-policy-content exception becomes a deny-only `Invalid` policy and may
allow startup; an unsafe filesystem object causes startup failure.

A future startup sequence should validate Den safety, anchor availability,
schemas, per-document commitments, and cross-document revision before making
network side effects. Detected stale, mixed, missing, or unverifiable protected
state should fail closed with distinct local diagnostics and no automatic
repair. `NotCommitted` may legitimately leave the previous durable generation;
it must not be mistaken for malicious rollback. After rename but before
confirmed directory durability, the outcome is indeterminate, not a safe
rollback or success. Any monotonic anchor protocol must specify exactly when
both state and anchor become durable and how to reconcile each crash window.

A single-file restore would violate a future manifest unless that restore is
explicitly re-committed by a local administrator. A whole-Den backup restore
would pass a Den-local manifest but conflict with an independent later anchor.
A VM/machine snapshot that includes all local anchors can still erase local
evidence. Bare-metal recovery may legitimately require explicit local
administrative reconciliation. Such an escape hatch must leave audit evidence,
never silently accept stale state, and never be reachable through remote Pack
membership. No recovery command is added here.

## Recommendation, claims, and next stage

Primary recommendation: **EXTERNAL_MONOTONIC_ANCHOR_REQUIRED** for meaningful
prevention of older valid state after whole-Den or full-machine restoration.
For Linux now, retain the existing strict local filesystem and fail-closed
semantics, and consider only a separately reviewed local consistency manifest
as a narrower defense against mixed/single-file restoration. Linux TPM or a
remote checkpoint could be studied later; neither is assumed available or
deployed. For future Windows support, keep the protected-state commitment and
anchor interface conceptually separate so the platform backend can differ,
without pretending a rollbackable OS store is monotonic.

Current answers: **Q1** no general single-file rollback detection; **Q2** no
whole-Den rollback detection; **Q3** no purely local same-image design reliably
detects full-machine rollback; **Q4** threat C when it includes all local state,
and D/E/F, require an independently surviving monotonic anchor for strong
freshness; **Q5** Linux can currently enforce Den access/crash consistency and
could add local mixed-state consistency, but not strong snapshot freshness
without an anchor; **Q6** a versioned canonical manifest plus a platform-neutral
anchor abstraction is portable in design, while its security depends on each
platform's actual anchor and recovery properties.

This stage does **not** claim rollback-proof, tamper-proof, snapshot-safe, or
that old policy cannot be restored. A secure file mode is not a freshness
proof; an in-memory authority epoch is not durable evidence. The proposed next
stage is a **Stage15B design review** of the manifest/transaction and external
policy-writer integration, with explicit fault-injection and migration cases.
Do not implement an anchor or policy format before that review.

## Validation and artifact boundary

The retained additions are test code and this document only; disposable
generated Pelt identities remain in temporary test Dens, which are removed by
the fixture. No raw captures, real user policy, private keys, traffic secrets,
payloads, or key logs are committed. The changed-file scan found no private
PEM block, TLS key-log line, or long embedded base64 literal:
`STAGE15_SECRET_MATERIAL_AUDIT = PASS`. The five new tests raise the Rust
aggregate from 156 to 161 (22 core unit, 12 core integration, 127 werewolfd);
Python remains 64 and acceptance remains 12/12. Formatting, Clippy, offline
workspace tests, Python tests, acceptance, and diff checks passed. Existing
Clippy warnings are outside the test-only change. Cargo.lock retains SHA-256
`09b18cac90690855e4bee28cabc59ee713f14baf50f2efbb20f2910ed46047b4`.
