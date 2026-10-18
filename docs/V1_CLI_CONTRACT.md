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
