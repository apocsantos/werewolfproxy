# Stage 15B — local persistent-state consistency manifest

## Scope and boundary

Stage 15B binds the security-relevant local policy set to one complete, durably selected generation. It protects `pack.json`, `silver.json`, `target_policy.json`, `fangs.json`, and `active_fangs.json` as one group. `pelt.json` remains outside the group: it has a create-only identity lifecycle and drives the Stage 12 runtime TLS identity publication. This work neither changes that lifecycle nor claims Pelt freshness.

The result detects an inconsistent local restoration relative to the selected manifest. It is not a freshness anchor. A complete historical Den generation, its matching selector, and all matching documents are internally consistent and may be accepted. The same applies to a complete VM or machine snapshot. Detecting those cases requires an anchor that the rollback adversary cannot restore.

## Threat model and protected group

The relevant local threat is a partial restore, corruption, or Den writer that leaves the selected policy set internally inconsistent. Stage 11B filesystem controls continue to protect against unprivileged unsafe path replacement, but permissions do not establish freshness. The group is exactly Pack, Silver, target policy, Fang profiles, and active Fangs. A Pelt rollback, full Den rollback, root/offline-disk replacement, and VM snapshot rollback remain outside this local-consistency mechanism.

## Storage and manifest schema

The Den contains two fixed regular-file slots, `a` and `b`. Each slot has the five protected documents and `security_state_<slot>_manifest.json`. The active slot is named only by the regular JSON selector `security_state_current.json`; `security_state_manifest_mode.json` is a separate mode marker. No symlink is used. Every file is opened and replaced through the Stage 11B `PrivateDirectory` descriptor, so normal Den ownership, mode, link, and atomic-write rules still apply.

The strictly parsed, canonical JSON records are conceptually:

```json
{"schema":1,"mode":"manifest"}
{"schema":1,"generation":42,"slot":"a"}
{"schema":1,"generation":42,"documents":{"pack":"…","silver":"…","target_policy":"…","fangs":"…","active_fangs":"…"},"state_commitment":"…"}
```

Unknown or duplicate fields, missing members, noncanonical encodings, malformed or non-lowercase-hex digests, unsupported schema, zero generation, and oversized records fail closed. The generation is a checked `u64`, starts at one, and refuses overflow.

## Canonical commitments

Documents are parsed into their typed semantic representation and emitted with deterministic struct order. Pack and Fang vectors are sorted by name; active profile names are sorted; target-policy peer keys and endpoint sets use ordered maps/sets. The canonical form is required on read, so whitespace, map-order, or duplicate-key ambiguity is rejected rather than hashed.

Each document digest is BLAKE3 over the explicit domain `werewolfproxy/state-manifest/v1/`, logical document name, schema byte, and canonical semantic bytes. The complete state commitment uses the distinct `werewolfproxy/state-manifest/v1/state` domain, schema, big-endian generation, and the five document digests in fixed order: Pack, Silver, target policy, Fangs, active Fangs. These are local storage domains; no Stage 10, TLS, QUIC, or wire-protocol domain changed.

## Generation semantics

Generation one is the initial migrated state. Every protected mutation uses checked `N + 1`; overflow fails closed. The generation is local diagnostic and consistency metadata only. It is not transmitted, negotiated, or treated as an external monotonic counter.

## Commit and publication model

For a protected mutation, the daemon builds a complete candidate from live state, validates it, writes all five canonical documents and the matching manifest to the inactive slot, then atomically replaces the current selector. The selector replacement, after all candidate files are durable, is the only activation point. Runtime publication follows durable selector publication. The two fixed slots bound storage without cleanup races; the prior slot remains as a recovery generation.

`NotCommitted` leaves the old selected generation active. An `IndeterminateAfterRename` marks storage degraded and rejects further administration; after restart the selected regular file determines whether the complete old or complete new generation is active. The deterministic fault matrix covers before and after each of the five document writes, manifest write, and selector write. Every restart is old generation active or new generation active; a mixed generation never activates.

Silver preserves its Stage 11C ordering. Silver ON fences runtime authority before its protected generation is persisted and stays fenced on persistence failure. Silver OFF commits the protected open state before `open_after_durable`. Pack revocation retains its existing authority fence before disk work. Target policy, Pack, Fang profiles, and active profiles use the same transaction once manifest mode is enabled.

### Stage 11B persistence interaction

Each write uses the existing `PrivateDirectory::replace` transaction semantics. A manifest slot is immutable from the active reader’s perspective, and the selector is a regular file rather than a symlink. `NotCommitted`, `DurablyCommitted`, and `IndeterminateAfterRename` retain their existing meanings; an indeterminate outcome degrades administrative mutation rather than attempting automatic repair.

### Pack, target policy, and Fangs

Every Pack mutation creates a candidate protected generation. Target-policy replacement is available only through the bounded local control request after migration. Fang profile changes and active-profile persistence use the same candidate and preserve active-to-profile cross-reference validation. Consequently an active Fang cannot refer to a profile from a different selected generation.

## Startup, migration, and recovery

Startup validates the Den and lock, then loads the marker and selector, strict manifest, documents, commitments, and cross-document active-profile references before creating runtime policy, control state, restored Fangs, or network listeners. A manifest-enabled Den fails closed on a missing selector or marker, malformed/incomplete slot, commitment mismatch, unsafe object, or ambiguous candidate.

Migration is an explicit local control operation: `state.manifest.migrate` (exposed as `werewolfctl state manifest-migrate`). It reads and validates legacy flat state, writes generation one to slot `a`, durably creates the marker, and finally creates the selector. Legacy loading remains only for a Den with no marker, selector, or slot artifact. Once artifacts exist, deleting either or both marker and selector cannot silently downgrade to flat loading. A complete restoration to a pre-migration Den still has no artifacts and is the stated whole-state rollback nonclaim. Post-migration target-policy edits use the same transaction through `target.policy.set` (`werewolfctl state set-target-policy '<JSON>'`); direct flat-file edits are ignored.

There is no automatic repair of a mismatch. An administrator must inspect the Den and restore one complete known-good generation/backup. A single protected-file or partial backup restore is rejected. A complete current or historical generation with its matching manifest and selector is accepted; this is deliberate local consistency rather than anti-rollback freshness.

### Rollback, backup, and recovery matrix

| Restore operation | Stage15B result |
| --- | --- |
| One selected protected document with current manifest | Reject before forwarding |
| Mixed selected documents from two generations | Reject before forwarding |
| Corrupted/missing selector, marker, member, or digest | Reject before forwarding |
| Complete current generation backup | Accept |
| Complete historical generation plus matching selector/manifest | Accept; freshness nonclaim |
| Complete Den or VM snapshot rollback | Accept if internally consistent; external-anchor nonclaim |

Recovery is explicit local administration: restore a complete known-good generation or repair the Den after inspection. The daemon never mints a replacement manifest from ambiguous files. A future external-anchor interface may record a selected generation and, after a separate design review, a Pelt fingerprint; it must provide an anchor the rollback attacker cannot restore.

## Tests and guarantees

Tests cover migration, deterministic canonical target-policy ordering, strict manifest parsing, generation overflow, every protected document replaced by valid prior-generation contents, current/mode deletion, complete historical selection, and each crash boundary. The mutation path test proves Pack, policy, profiles, and active profiles advance one shared generation and no longer write legacy flat files after migration. Existing Stage 9–14 regressions remain in the normal workspace suite.

The supported claim is: **WerewolfProxy binds its security-relevant local policy state to a single durably committed local generation and rejects inconsistent or partial protected-state restoration before network forwarding becomes active.**

It does not claim whole-Den rollback protection, VM snapshot rollback protection, root/offline freshness protection, or a monotonic freshness guarantee without an external non-rollbackable anchor. Pelt rollback is also outside this first group. A later external-anchor interface could bind a selected generation and, after separate design review, a Pelt fingerprint.

`STAGE15B_WIRE_FORMAT_CHANGE = NONE`.
