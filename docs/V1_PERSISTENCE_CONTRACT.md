# WerewolfProxy v1 RC persistence contract

## Den and identity

The resolved Den remains protected by Stage11B ownership, modes, no unsafe
symlink/hardlink objects, an exclusive OS lock, descriptor-relative file
operations, and atomic durable replacement. User-mode and system-service Den
paths are explicit. Runtime files under /run are not durable state.

Pelt is an Ed25519 identity initialized explicitly and create-only. The
fingerprint is derived from its public key; public identity export never emits
the private seed/key. Pelt is not automatically rotated and is not TOFU. Its
public identity is pinned as the exact SPKI for TLS receiver authentication.
Pelt is intentionally outside the Stage15B protected consistency group.

## Independent Den state (Stage23–31)

These files live in the Den beside the protected group, outside it. Each
is written atomically and durably before the change is published (persist
first, then publish), and none grants anything: Pack, Target Policy,
Silver and governance still decide every access.

| File | Stage | Content | Malformed at startup |
|---|---|---|---|
| `services.json` | 23, 27 | Service Registry: name, peer, target, kind, optional `fallbacks` | fails closed |
| `advertise.json` | 24 | `{"version":1,"addresses":[...]}`, the node's own advertised addresses | fails closed |
| `contacts.json` | 24 | Cache of verified contact records, at most 256; only a hint, re-verified on use | ignored (whole file), invalid or expired records dropped |
| `published.json` | 25 | Published services, at most 32 | fails closed |
| `routes.json` | 26 | Routes: peer, relay, hop; at most 256 | fails closed |
| `governance_NNNNNNNN.wwgov` | 28 | Archive copy of every applied statement, by sequence; written before its head is committed, served by sync only if it matches the committed head | — |
| `auto_sync.json` | 31 | The automatic-sync switch (spec decision S1): exactly `{"version":1,"enabled":true\|false}`, absent meaning enabled | fails closed |
| `tombstone.json` | 30 | The Tombstone, after death or retirement: strict canonical JSON, at most 8 KiB | the daemon exits 65 |

Stage31's switch only decides whether this node fetches governance
statements on its own.

## Protected generation

The Stage15B group is Pack, Silver, target policy, Fang profiles, and active
Fang intent. Two fixed slots, a and b, each hold canonical document files and
security_state_<slot>_manifest.json. The regular-file selector
security_state_current.json names one slot and generation. The separate
security_state_manifest_mode.json marker commits the Den to manifest mode.
The selector is not a symlink.

Manifest schema is 1. A manifest records generation, five lower-case
hexadecimal BLAKE3 document commitments, and an overall state commitment.

**Schema 2 (Stage28).** Applying a first governance genesis adds a sixth
protected document, `security_state_<slot>_governance.json`, the
governance head: network ID, sequence, head hash, threshold, key epochs,
revoked keys, the hash of every applied statement, the fork flag, and
from Stage29–30 the roster, `activated_at` and `dead_at`. The manifest
then has schema 2 and six commitments, in the same generation as the
Pack it bounds. Binaries keep reading schema 1; 1.0.0-rc.1 and earlier
binaries refuse schema 2, and a node never goes back to schema 1.
Canonical typed semantic serialization and fixed field order are used. The
generation starts at 1 and checked-increments for each successful protected
mutation; overflow fails closed. Domain-separated local commitments do not
alter wire crypto.

The inactive slot is written and durably validated before the regular selector
is atomically replaced. Selector publication is the durable commit boundary;
runtime publication follows it. Silver ON first advances the authority fence
and cancels sessions, then persists locked state. A failed persistence leaves
the runtime fenced. Silver OFF publishes open authority only after durable
commit and healthy storage.

Legacy migration is explicit via werewolfctl state manifest-migrate. Once
manifest mode artifacts exist, missing or malformed selector/marker never
silently falls back to legacy flat files. Startup checks the selected complete
generation and cross-object Fang references before forwarding listeners.

## Guarantee and nonclaim

Stage15B binds these local policy files to one selected generation and rejects
partial or inconsistent restoration before forwarding. It does not prove
freshness against restoration of a complete, internally consistent historical
Den or whole-machine snapshot. A same-disk manifest, hash chain or selector
can roll back with the state. A future non-rollbackable external anchor would
be a separate design and is not required for v1.
