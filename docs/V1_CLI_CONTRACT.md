# WerewolfProxy v1 RC CLI contract

This contract freezes the visible Stage18 administrative commands. The broad
online/offline behavior and output contract are RC interfaces, not a promise of
indefinite compatibility after v1.

Global options: werewolfctl [--socket PATH] [--home PATH] [--json]. Explicit
socket selects the daemon. Offline commands use --home; online mutations go
through the selected daemon's bounded same-UID control socket. There is no
independent CLI writer behind a running daemon.

Supported commands:

- den init, den info
- pelt init, pelt show
- pack add NAME PUBLIC_KEY_B64 ADDRESS; pack list, remove NAME, revoke NAME,
  set-address NAME ADDRESS
- target allow PEER IP:PORT; target list; target remove PEER IP:PORT
- fang create NAME PEER LOCAL REMOTE [--transport quic|tcp|tcp-plain]; fang
  list, active, activate NAME, deactivate NAME, remove NAME
- silver status, on, off
- status, health, diagnose, doctor
- state manifest-migrate, backup-info

Fang's transport argument selects quic, secure TCP, or explicit tcp-plain for
that profile; it is not a fallback selector. The local Stage11A automatic
selector defaults to secure (an alias for strict): QUIC secure, encrypted TCP
secure, then failure. compatibility has the same secure order unless the
invocation explicitly supplies allow-plain-fallback. LEGACY deliberately
retains the historical weaker fallback behavior. Plain mode is an explicit
local choice and never a remote capability or downgrade negotiation. The
werewolfctl profile surface does not make plain fallback implicit.

`pelt show` exports public key and fingerprint only. Pack add derives and
validates identity from public key material. Target rules are exact literal IP
and port grants; no hostname, CIDR, wildcard or range grants. Protected
mutations serialize in the daemon and commit through Stage15B. Hidden legacy
commands are not supported v1 administration.

## JSON and exit status

With --json, stdout is exactly one JSON object with stable Stage18 top-level
fields ok, command, and result. Human diagnostics go to stderr. The broad exit
classes are 0 success, 64 usage/input, 65 local security-state failure, 66
conflict/already-running, 69 daemon unavailable, and 70 locally rejected
operation. Not-found/conflict detail remains in the local diagnostic/result;
no secret material is returned.

Systemd is controlled separately with systemctl; werewolfctl does not shell
out to it. The CLI has no network pairing or remote administration feature.

## Stage23 extension: named services

Stage23 adds a Service Registry after the RC freeze. It is an additive
online command group; the commands and exit classes above are unchanged.

- service add NAME PEER TARGET tcp|http|https|ssh
- service list; service show NAME; service remove NAME
- service connect NAME --local-port PORT [--bind ADDR] [--transport
  quic|tcp|tcp-plain]

The registry records intent only: a name, a Pack peer name, a target on that
peer and a kind label. It grants no access. `service add` does not require the
peer to be in the Pack yet, and the kind does not change transport behavior.
Fields must be non-empty and canonical (no surrounding whitespace); names are
unique and lookups are exact. TARGET is stored as given (normally IP:PORT)
and checked only when a Fang opens. Definitions persist in the Den as `services.json`
(persist first, then publish), outside the Stage15B protected manifest.

`service connect` sends only the name, local listener and transport. The
daemon resolves the name to its registered peer and target and opens the Fang
through the same path as `fang open`, so Pack membership, Pelt, Silver and the
receiver's deny-by-default Target Policy apply unchanged. The request cannot
override the peer or target. `--bind` defaults to 127.0.0.1 and `--transport`
to quic; tcp-plain stays an explicit choice.

Service mutations and `service connect` serialize with other protected
mutations and are refused while storage is degraded; Silver refuses
`service connect` like any Fang open. The daemon reports invalid, duplicate,
not-found and failed-save outcomes as `SERVICE_INVALID`, `SERVICE_EXISTS`,
`SERVICE_NOT_FOUND` and `SERVICE_SAVE_FAILED`; werewolfctl exits 70 with the
message (with --json, `ok:false`, `error`, `exit_class`). A failed save leaves
the previous registry live. The disposable gate
`tests/integration/service_gate.py` exercises this contract.

## Stage24 extension: discovery

Stage24 adds an online `discover` group
([design](STAGE24_FINDNODE_DESIGN.md), wire messages in
[V1_WIRE_CONTRACT.md](V1_WIRE_CONTRACT.md)). Earlier commands and exit
classes are unchanged.

- discover advertise ADDRESS... (1 to 4 `quic://IP:PORT` or `tcp://IP:PORT`)
- discover withdraw
- discover show; discover contacts
- discover refresh [--transport quic|tcp]
- discover find PUBLIC_KEY_B64|NODE_ID_HEX [--transport quic|tcp]

Discovery answers where a node is, never whether to trust it. No command
changes the Pack, Target Policy or authority; reaching a discovered node still
needs `pack add` on both sides and the receiver's `target allow`.

`advertise` validates the addresses (literal IPs only, no hostnames), persists
them as `advertise.json` and then publishes them; the daemon signs its own
record from them with `seq` taken from the clock and re-signs it when the
addresses change or less than a day of its seven-day validity is left.
`withdraw` clears them. Both serialize with other mutations and are refused
while storage is degraded.

`refresh` pings every Pack member for its own record. `find` is an iterative
lookup: it starts at the Pack members nearest to the target, asks three at a
time, then asks the nodes they return, dialling each at its advertised address
with TLS pinned to its record key, until the target's record arrives (at most
8 hops and 24 queries). A hop answers only if the requester is in its Pack.
Both finish within seven seconds and report a status per node asked (`ok`
with a record count, `unreachable`, `rejected`, `timeout`, `not_queried`,
`bad_address`, `bad_route`; `find` adds the `hop`, and names nodes outside the Pack
`node:<id prefix>`),
and are refused under Silver
(`SILVER_ACTIVE`) and while another refresh or find runs (`DISCOVER_BUSY`).
`unreachable` means no connection was made (refused, no route, or no
answer to the dial); `rejected` means the node was reached and the
handshake, authentication or result check failed. Verified records go to
the cache `contacts.json` (at most 256, highest `seq` per node), written
before it is published; a malformed cache is ignored
rather than blocking startup. A node answers FindNode only with cached records
of its own current Pack members.

Errors are `DISCOVER_INVALID`, `DISCOVER_ADDRESS_INVALID`,
`DISCOVER_SAVE_FAILED`, `DISCOVER_BUSY`, `SILVER_ACTIVE` and `NO_PELT`;
werewolfctl exits 70. The disposable gate `tests/integration/discovery_gate.py`
exercises this contract with four wolves.

## Stage25 extension: service discovery

Stage25 ([design](STAGE25_SERVICE_DISCOVERY_DESIGN.md), wire rules in the
[services RPC spec](STAGE25_SERVICES_RPC_WIRE_SPEC.md)) adds:

- service publish NAME TARGET tcp|http|https|ssh; service unpublish NAME;
  service published
- discover services [--peer NAME] [--transport quic|tcp]
- service add NAME --from PEER [--transport quic|tcp]

`publish` offers a local target under a name. It grants nothing: a Pack
member sees the entry only while this node's Target Policy grants it that
exact target, the same decision OPEN applies, so revoking a grant hides the
entry at once. NAME follows the Stage23 rules and may not contain control
characters; TARGET must be a canonical literal `IP:port` (no hostnames,
non-zero port, specified address); at most 32 services. Publications persist as
`published.json` (persist first, then publish; outside the Stage15B
manifest; malformed fails startup closed, like `services.json`).
`published` lists each entry with the Pack members it is currently visible
to.

`discover services` asks every Pack member (or only `--peer`) and reports
per member `ok` with its entries, `unreachable`, `rejected`, `timeout`,
`bad_address`, `bad_route` or `not_queried`, within seven seconds. An empty list does not distinguish
"nothing published" from "nothing granted to you". It stores nothing.

`service add NAME --from PEER` asks Pack member PEER at that moment for the
services it shows this node and, if NAME is among them, adds it to the
Service Registry with PEER and the received target and kind, through the
ordinary `service add` path (persist first, refused while storage is
degraded). The query runs before mutation admission, so it never blocks
other mutations. It never copies from a cache, and the resulting entry grants
nothing more than a manual `service add`.

Under Silver, `discover services` and `--from` are refused. Errors are
`SERVICE_INVALID`, `SERVICE_EXISTS`, `SERVICE_NOT_FOUND`,
`SERVICE_SAVE_FAILED`, `DISCOVER_INVALID`, `DISCOVER_FAILED` (PEER
unreachable or rejected the query), `SILVER_ACTIVE` and `NO_PELT`;
werewolfctl exits 70. The disposable gate
`tests/integration/service_discovery_gate.py` exercises this contract.

## Stage26 extension: routes

Stage26 ([design](STAGE26_MULTIHOP_ROUTING_DESIGN.md)) lets a Fang reach a
Pack member through another Pack member (the relay).

- route set PEER --via RELAY [--hop quic|tcp]
- route list
- route remove PEER

A route says that Pack member PEER is reached through Pack member RELAY,
with `--hop` (default `quic`) the transport from this node to RELAY; the
session inside it, to PEER, is always encrypted TCP pinned to PEER's Pack
key. A route grants nothing: RELAY forwards only if its own Target Policy
grants this node PEER's listener address, and PEER's Target Policy decides
access as on a direct Fang.

PEER and RELAY must both be Pack members when the route is set; `set`
replaces PEER's existing route. Exactly one relay: a relay cannot itself be
routed and a routed peer cannot relay. At most 256 routes. Routes persist in
the Den as `routes.json` (persist first, then publish), outside the Stage15B
protected manifest, and mutations serialize with other mutations and are
refused while storage is degraded. Errors are `ROUTE_INVALID` (unknown
member, self or chained relay, bad hop), `ROUTE_NOT_FOUND` and
`ROUTE_SAVE_FAILED`; werewolfctl exits 70. A failed save leaves the
previous routes live.

A Fang (`fang open`, a profile, or `service connect`) to a routed PEER opens,
for each local connection, an ordinary session to RELAY over the route's hop
whose target is PEER's encrypted-TCP listener, and inside it the unchanged
encrypted-TCP session to PEER. The route is resolved when the Fang opens:
RELAY must still be a Pack member with an address for the hop, and PEER's
Pack address must be a literal `IP:port` (a `quic://` address is dialled as
TCP on the same port, as for direct Fangs); otherwise the open fails with
`FANG_ROUTE_INVALID` or `FANG_BAD_TCP_ADDRESS`. `--transport tcp-plain` to a
routed PEER fails with `FANG_ROUTE_INVALID`; `quic` and `tcp` select nothing
for a routed PEER, because the route's hop does. The open reports `via` and
`peer_addr` (PEER's listener), and `fang active` shows the transport as
`tcp via RELAY (HOP)`. Any failure at either hop resets the local
connection. A routed Fang depends on RELAY as on PEER: changing RELAY's
address closes it and deactivates profiles routed through RELAY (as for
direct profiles), and revoking or removing RELAY closes it. Changing or
removing a route applies to Fangs opened afterwards. The disposable gate `tests/integration/routed_fang_gate.py`
exercises this contract.

Discovery follows routes too: `discover refresh`, `discover find` (for its
Pack seeds), `discover services` and `service add --from` reach a routed
PEER through RELAY with the same RPC over encrypted TCP, pinned to PEER's
key, needing the same RELAY grant as a routed Fang; `--transport` does not
apply to a routed PEER. A route that no longer resolves is reported as
`bad_route`; the member is never dialled directly instead. Nodes learned
from contact records are always dialled directly.

## Stage27 extension: service providers

Stage27 ([design](STAGE27_PROVIDER_SELECTION_DESIGN.md)) lets one service
entry name an ordered list of providers.

- service provider add NAME PEER TARGET
- service provider add NAME --from PEER [--transport quic|tcp]
- service provider remove NAME PEER

A service's providers are its first `peer`/`target` followed by its
fallbacks, in the order added; `service show` lists them. `provider add`
appends one; `--from PEER` takes the target from NAME as PEER publishes it
to this node at that moment (as `service add --from`) and is refused if
PEER's kind label differs from the entry's. `provider remove` drops one;
removing the first promotes the next, and the last provider goes only with
`service remove`. At most 4 providers, none listed twice; PEER need not be
in the Pack yet. Providers grant nothing: each still authorizes the
requester with its own Target Policy. Entries persist in `services.json`
with an optional `fallbacks` list, omitted when empty, so an entry with one
provider keeps its Stage23 form. Errors are `SERVICE_INVALID`,
`SERVICE_NOT_FOUND`, `SERVICE_SAVE_FAILED` and the `--from` discovery
errors; werewolfctl exits 70. `--transport` without `--from` is a usage
error (exit 64) for `service add` and `service provider add`.

`service connect NAME` on an entry with fallbacks opens one local listener
whose every connection tries the providers in order, each through its
ordinary session (direct, or through its Stage26 route). A provider is
skipped only if it fails before its session is established (unreachable,
TLS pin mismatch, OPEN refused); once its ACK has verified, any failure
resets the local connection and no other provider is tried. The selection
for one connection has 15 seconds; if every provider fails, or the time
runs out, the local connection is reset. Every provider must resolve when
the Fang opens (a Pack member with an address for `--transport`, or a valid
route), otherwise the open fails (`FANG_UNKNOWN_PEER`, `FANG_BAD_ADDRESS`,
`FANG_BAD_QUIC_ADDRESS`, `FANG_ROUTE_INVALID`); `--transport tcp-plain` is
refused (`FANG_INVALID`). The open reports `providers` in order and `fang
active` shows `providers: A, B, ...`. Changing, revoking or removing any
listed provider, or a provider's relay, closes the Fang; provider list
changes apply to Fangs opened afterwards. The disposable gate
`tests/integration/provider_gate.py` exercises this contract.

## Stage28 extension: governance files

Stage28 ([survey](STAGE28_GOVERNANCE_SURVEY.md),
[statement spec](STAGE28_GOVERNANCE_STATEMENT_SPEC.md)) adds network
governance statements signed by at least 7 of 10 governance keys.

- governance keygen OUT
- governance draft genesis --key PUB_B64 (×10, index order) [--network-id HEX] --out FILE
- governance draft rotate --after PREVIOUS --set INDEX=PUB_B64 ... --out FILE
- governance draft revoke --after PREVIOUS --member PUB_B64 ... --out FILE
- governance draft admit --after PREVIOUS --member PUB_B64 ... --out FILE (Stage29)
- governance draft activate --after PREVIOUS --out FILE (Stage29)
- governance sign DRAFT KEYFILE --index N --out PART
- governance combine DRAFT PART... --out FILE
- governance inspect FILE [--chain FILE...] [--expect-genesis HEX]

These run locally and never contact a daemon or the network, so a key
holder can use them on an offline machine. `keygen` creates a key file with
mode 0600 and never replaces an existing file; `sign` refuses a key file
readable by anyone but its owner or whose public key does not match its
secret. `draft rotate` and `draft revoke` follow the statement given with
`--after` (the current chain head) and sort their entries. A part records
the hash of the draft it signs, so `combine` refuses parts for another
statement or two parts for one index. `inspect` prints a statement and its
hash; with `--chain` (genesis first) or `--expect-genesis` it verifies the
chain exactly as a node will, pinning the genesis to the given hash or, if
none is given, to its own hash, which the operator must compare out of
band. Output files are never overwritten. Errors exit 70, usage errors 64.

On the node:

- governance apply FILE [--expect-hash HEX]
- governance status

`apply` reads the statement file locally and sends it to the daemon, which
verifies it against its governance head exactly as the spec defines and,
if it is the next statement, commits the new head in the protected manifest
before acting on it (persist first, then publish). A genesis is applied
only with `--expect-hash` equal to its hash, exchanged out of band (S1);
governance needs a manifest-enabled Den (`GOVERNANCE_NEEDS_MANIFEST`, run
`state manifest-migrate` first), and the first genesis moves the manifest
to schema 2, which older binaries refuse (S2). A network revocation acts
like `pack revoke` on every Pack member holding a revoked key: runtime
authority is denied first, every Fang to it or through it (relay or
provider) is closed, and the denial stays if the save fails
(`GOVERNANCE_SAVE_FAILED_RUNTIME_DENIED`); afterwards `pack add` of a
revoked key fails with `PACK_REVOKED_BY_NETWORK`. A key rotation changes
which keys sign later statements. Re-applying an applied statement reports
`already_applied`. A statement out of order, for another network, badly
signed or breaking a body rule fails with `GOVERNANCE_REJECTED`; a validly
signed statement that conflicts with an applied one is recorded durably
and fails with `GOVERNANCE_FORK`, after which every statement fails with
`GOVERNANCE_FORKED` (S3). Applied statements are also copied to
`governance_NNNNNNNN.wwgov` in the Den for audit. `status` shows the
network ID, sequence, head hash, threshold, current keys, revoked keys and
the fork flag. Errors exit 70. The disposable gate
`tests/integration/governance_gate.py` exercises this contract.

Stage29 ([design](STAGE29_ACTIVATION_DESIGN.md),
[addendum](STAGE29_GOVERNANCE_ADDENDUM_SPEC.md)) adds admission and
activation statements. Activation is a network-chain fact; admission is
local eligibility derived from that chain. Every node applies a valid
activation. In the same protected generation it removes every Pack member
whose key is not on the roster or is revoked, exactly as a revocation does,
and reports them in `revoked_members`. After activation `pack add` of a key
not on the roster fails with `PACK_NOT_ON_ROSTER` (a revoked key still with
`PACK_REVOKED_BY_NETWORK`). A node whose own Pelt is not on the roster or
is revoked is **unadmitted**. While unadmitted:

- Fang, profile and `service connect` opens fail with
  `GOVERNANCE_UNADMITTED`;
- discovery commands fail with the same code;
- inbound sessions and RPCs are refused, using the same lock as Silver;
- open Fangs close when it becomes unadmitted.

`governance apply/status` keep working. A later admission of its Pelt lifts
the fence automatically. `silver off` does not lift it.

`governance status` adds `roster`, `activated_at` and `admitted`, and
`apply` reports `admitted`. An activated node whose Pack holds a key the
head excludes refuses to start. The disposable gate
`tests/integration/activation_gate.py` exercises this contract.

- governance sync [--peer NAME] [--transport quic|tcp]

`sync` (Stage29, [RPC spec](STAGE29_GOVERNANCE_RPC_WIRE_SPEC.md)) asks every
Pack member, or only `--peer`, for the statements after this node's head
with the read-only `governance` RPC. A routed member is asked through its
relay. For each next sequence it verifies every offered candidate
independently:

- one valid candidate: applied exactly as by `governance apply`, with all
  its effects, then the next sequence;
- two different valid candidates: the fork is recorded and nothing more is
  applied (`stopped` with `GOVERNANCE_FORK`);
- no valid candidate: it stops.

It runs up to 16 rounds of 4 statements and reports, per peer, its status
and how many statements it offered in each round, and the statements
applied. Each applied statement is archived before its head is committed; a
responder serves only archive entries matching its committed head.

Refusals:

- under Silver (`SILVER_ACTIVE`);
- without a Pelt (`NO_PELT`);
- without a governance head (`GOVERNANCE_NO_GENESIS`; a genesis is only
  ever applied from a file);
- after a fork (`GOVERNANCE_FORKED`);
- while another discovery runs (`DISCOVER_BUSY`).

It is manual only; nothing syncs automatically. The disposable gate
`tests/integration/sync_gate.py` exercises it.

