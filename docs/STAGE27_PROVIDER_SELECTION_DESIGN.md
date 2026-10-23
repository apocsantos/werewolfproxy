# Stage27 provider selection — design proposal

**Status: proposal, awaiting owner decisions 1–5 below.** Nothing here is
implemented, and no option changes a handshake, transcript or wire format.

This is §20 stage 5 of [ARCHITECTURE.md](ARCHITECTURE.md) ("routing to a
selected provider") and the provider choice sketched in §47: after Stage25
a node can see that several Pack members offer a service, but a registry
entry still names exactly one of them.

## Where we are

A Stage23 registry entry is `{name, peer, target, kind}`.
`service connect NAME` opens one Fang to that one peer, possibly through a
Stage26 relay. If that peer is down, refuses, or has revoked the grant,
every connection fails until the operator edits the entry. Stage25
`discover services` can show that `wolf-b` and `wolf-f` both publish `web`,
and `service add web --from wolf-b` copies one of them; nothing uses the
second.

## Goals and non-goals

- One registry entry can name an ordered list of providers, each a Pack
  member with its own target, and a connection uses the first that accepts.
- Each provider still authorizes independently (Pelt pin, signed OPEN,
  its own Target Policy); choosing a provider never widens access (§13,
  §49).
- Application bytes are never replayed: a provider is abandoned only
  before the session is established.
- Out of scope: load balancing, health scoring or reputation (§48),
  latency measurement, automatic discovery of providers at connect time,
  more than one session per local connection, and moving a live session to
  another provider.

## Proposed model

### Who may be a provider (decision 1)

**1a. An operator-chosen list (recommended).** The entry lists its
providers explicitly; `service provider add NAME PEER TARGET` appends one,
and `--from PEER` may fill in the target from that peer's Stage25 answer.
Nothing else ever becomes a provider. **1b. Any Pack member currently
publishing the name.** Convenient, but a service name is only a label each
provider chooses, so any Pack member could attract the traffic by
publishing `web` (the service-name poisoning §50 warns about). 1b would at
least need provider pinning per name; 1a needs nothing extra.

### When a provider is chosen (decision 2)

**2a. Per local connection, before any application byte (recommended).**
For each accepted local connection the Fang tries the providers in order:
open the session (directly or through the provider's route) and, only once
that provider's ACK has verified, start copying. A failure before that
point (dial refused or timed out, TLS pin mismatch, OPEN refused, target
unreachable at the provider) moves to the next provider; a failure after
it is a session failure, reset to the application as today, never retried.
**2b. Once at Fang open.** The first provider that accepts a probe serves
the Fang for its lifetime; simpler, but a provider failing later breaks
every new connection until the Fang is reopened. **2c. At import only.**
What exists today.

### Order (decision 3)

**3a. Operator order (recommended):** deterministic and auditable; the
second provider sees traffic only when the first fails. **3b. Rotate**
across providers per connection to spread load; it reveals every
connection pattern to every provider and makes failures harder to
diagnose.

### Bounds (decision 4)

**4a (recommended).** At most 4 providers per entry. Each attempt is
bounded by the existing handshake deadlines; the whole selection for one
local connection has a 15 s budget (`CLIENT_WINDOW`), after which the
local connection is reset. Attempts are sequential, so at most one session
per local connection is ever being established. **4b.** Parallel attempts
("happy eyeballs"): faster failover, but every provider sees an OPEN for
every connection and the losers must be torn down.

### Storage (decision 5)

**5a (recommended).** `services.json` keeps today's `peer` and `target` as
the first provider and gains an optional `fallbacks` list of
`{peer, target}`, omitted when empty, so an entry without fallbacks is
byte-identical to today's and older files load unchanged. A file with
`fallbacks` is rejected by older binaries (strict parsing), which is the
safe direction. **5b.** Replace `peer`/`target` with a `providers` list: a
format migration for no functional gain.

## Operator surface (draft, for 1a–5a)

```text
werewolfctl service provider add NAME PEER TARGET | NAME --from PEER
werewolfctl service provider remove NAME PEER
werewolfctl service show NAME          # providers in order
werewolfctl fang active                # which provider served the last connection
```

`service connect NAME` is unchanged; with fallbacks it opens one local
listener that tries the providers per connection. Errors follow the
Stage23 classes (`SERVICE_*`); an entry may not list a peer twice.

## Security notes

- **No new authority path.** Every attempt is the normal Fang open to that
  provider (or its routed form); each provider's Pelt pin, signed OPEN and
  Target Policy apply unchanged. A registry list is intent, never a grant.
- **No replay.** Failover happens only before the provider's ACK is
  verified; no application byte has been read from the local socket by
  then, so nothing is sent twice. After the ACK, failure is reported as
  today.
- **Metadata.** A fallback provider learns that the requester tried to
  connect, only when the earlier ones failed (3a). An OPEN refused by one
  provider is not revealed to the next.
- **Failure signalling.** If every provider fails, the local connection is
  reset, never closed cleanly.
- **Interaction with routes and Pack changes.** A provider's route applies
  to its attempts. Revoking, removing or re-addressing a provider closes the
  Fang as for any of its peers (LC-01 and the Stage26 relay rule extended to
  every listed provider).

## Proposed steps

| Step | Scope |
|---|---|
| 0 | This proposal; owner decisions 1–5 |
| 1 | `werewolf-core`: `fallbacks` in the service definition, validation (Pack names, unique peers, at most 4), strict JSON, unit tests |
| 2 | CLI and control: `service provider add/remove`, `--from` for a provider, `service show`; CLI contract |
| 3 | Fang: per-connection ordered attempts for direct, QUIC, TCP and routed providers; Pack changes close Fangs naming any provider; lab gate with three providers (first down, first refusing, second's grant revoked, all failing) |
| 4 | Real-WAN check |

## Decisions

Pending. Recommended: **1a, 2a, 3a, 4a, 5a.**
