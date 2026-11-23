# Stage56 M2 — optional credential-bearing local Fang endpoint

**Status: accepted 2026-11-23 by the merge of PR #180; implemented in M3.**
Stage56 M1 confirmed that the existing transparent TCP
listener cannot identify local users. This proposal evaluates an additive
Linux AF_UNIX endpoint for operators who need per-UID admission limits while
retaining all existing TCP Fangs and clients unchanged.

## Proposed contract

An operator may choose a local endpoint of the form `unix:/absolute/path`
when opening a Fang or saving a Fang profile. Existing `IP:port` values keep
their current meaning and behavior. One Fang has exactly one local endpoint;
the selected endpoint is either TCP or Unix stream, never both. Applications
using the Unix endpoint connect to that socket and continue to send the same
unmodified target byte stream. No peer wire message, receiver identity,
authorization rule, remote route or persistence format changes.

On Linux, the accept path reads the connected socket's peer UID from kernel
credentials and uses it as the local share key. The proposed share ceiling is
the existing **16 concurrent slots per source**. When that UID reaches its
share, only one useful own-UID idle session may be replaced under the current
60-second idle rule; an active connection is never preempted. A different UID
can still use the Fang's remaining local and global capacity. Total limits
remain **64 per Fang / 1024 per daemon**. Legacy TCP loopback keeps its current
shared behavior because it has no UID key.

The endpoint requires an operator-managed existing parent directory. The
socket is created with owner/group access only (proposed mode `0660`); the
directory's group and permissions determine which local users can connect.
The application does not create or chmod parent directories. Startup fails
closed if the endpoint cannot be bound or secured. On shutdown, the daemon
removes only the socket filesystem object it created, after verifying its
identity has not been replaced. A stale socket after a crash is reported for
operator cleanup; the daemon never blindly unlinks an existing path.

## Listener and stream architecture

The acceptor is currently concrete over `TcpListener`/`TcpStream`; QUIC,
encrypted TCP, plain TCP, routed and failover paths pass `TcpStream` through
their setup and copy functions. The implementation would introduce an internal
`FangListener` / `FangClientStream` abstraction with TCP and Unix variants and
an admission identity enum for an IP prefix or UID. It must preserve TCP's
reset-on-rejection behavior. A rejected Unix client observes an immediate
stream close, since Unix streams have no TCP RST. Unix I/O goes directly into
the existing transport setup and copy paths; it is not proxied through a
second TCP connection.

A socket-to-TCP bridge is rejected. Each bridged session needs the Unix client,
the bridge's TCP client and the Fang's accepted TCP socket. That adds about two
descriptors per session over the direct listener path. The M3 full-cap TCP
baseline used about 2,096 descriptors for 1,024 sessions; adding 2,048 would
reach about 4,144, above the task's 4,096 descriptor limit before headroom.
The direct-stream design avoids this per-session descriptor multiplier.

The generic stream change must cover all current local accept paths: QUIC,
encrypted TCP, plain TCP, routed and failover. It must keep transport-specific
socket options on TCP only, preserve caller reset/cancel behavior, pass the UID
identity into the shared Fang admission registry, and retain receiver/global
slot cleanup on every exit path.

## Compatibility, platform and validation

- Existing TCP listener values, command lines, profiles, active records and
  clients stay byte-for-byte compatible. A new `unix:` value is parsed only
  where Unix listeners and peer credentials are supported; unsupported
  platforms reject it clearly and continue to support TCP Fangs.
- Linux is the first supported identity target because this contract depends
  on `SO_PEERCRED`. Other Unix variants require separate evidence before
  enabling the endpoint. The current release target remains Linux x86_64.
- Tests must cover old TCP profiles and Fangs unchanged, Unix path validation
  and permissions, actual peer UID attribution, per-UID share limits and idle
  replacement, distinct UIDs sharing one Fang, active-session protection,
  stale/replaced socket cleanup, cancellation, all five transports, descriptor
  bounds, target authority, daemon survival and cleanup.
- A native multi-UID lab must run in a private user/network namespace where
  available. Unit tests can inject identities for deterministic policy tests;
  they do not substitute for one test of real kernel credentials.
- No claim follows for Windows named pipes, non-Linux Unix credentials,
  container namespace UID mapping, a TCP client fairness guarantee, WAN
  behavior or application-level idleness.

## Decisions recorded by PR #180

| ID | Decision | Proposal |
| --- | --- | --- |
| S56-D0 | Add an optional local identity-bearing endpoint? | Yes, if the owner accepts a new local interface; keep TCP unchanged |
| S56-D1 | UID share | Fixed 16 slots per UID, matching the existing non-loopback source ceiling |
| S56-D2 | Endpoint access | Operator-managed parent directory; owner/group socket mode `0660`; fail closed |
| S56-D3 | Platform | Linux only until another platform has peer-credential evidence |
| S56-D4 | Scope | All local Fang kinds through one stream abstraction; no TCP proxy bridge |
| S56-D5 | Compatibility | Keep existing TCP values/defaults and raw byte-stream semantics unchanged |
| S56-D6 | Authorize implementation and tests? | Accepted in PR #180; implementation and tests merged in PR #181 |

The M1-recommended TCP-only disposition would have remained in effect if D0
or D6 had not been accepted. The Linux `SO_PEERCRED` scope is described by
[unix(7)](https://man7.org/linux/man-pages/man7/unix.7.html).
