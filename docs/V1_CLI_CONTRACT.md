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
with a record count, `rejected`, `timeout`, `not_queried`, `bad_address`;
`find` adds the `hop`, and names nodes outside the Pack `node:<id prefix>`),
and are refused under Silver
(`SILVER_ACTIVE`) and while another refresh or find runs (`DISCOVER_BUSY`).
Verified records go to the cache `contacts.json` (at most 256, highest `seq`
per node), written before it is published; a malformed cache is ignored
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
per member `ok` with its entries, `rejected`, `timeout`, `bad_address` or
`not_queried`, within seven seconds. An empty list does not distinguish
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
