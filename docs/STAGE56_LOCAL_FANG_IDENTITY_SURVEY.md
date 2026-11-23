# Stage56 M1 — local Fang client identity and fairness

**Status: M1 source review complete; no runtime change proposed for the
existing TCP interface, 2026-11-23.** This survey follows the active-loopback
flood limitation retained by [Stage43](STAGE43_CLOSURE.md). It evaluates
whether a Fang can enforce per-local-user admission fairness while preserving
its current transparent TCP listener. No M2 implementation or policy change
is authorized by this survey.

## Finding

The Stage43 M1/M3 labs established that active loopback clients can occupy all
64 local Fang slots and cause other clients to reset until capacity is
released. Idle eviction cannot help: active connections are not idle. Stage55
adds a global-capacity trigger for one useful established idle session; it
does not preempt active sessions. See the [Stage43 results](STAGE43_M3_REGRESSION.md),
[Stage55 results](STAGE55_M3_REGRESSION.md) and the current
[limitations](../KNOWN_LIMITATIONS.md).

The current acceptor uses `tokio::net::TcpListener`. `FangAcceptor::accept`
receives a `TcpStream` and `SocketAddr`; its source policy derives a key from
the remote IP address. `Policy::share_key` deliberately returns no source key
for loopback. Thus separate local processes and Unix users connecting over
127.0.0.1 or ::1 are indistinguishable to the current admission policy. A
connection does not carry local process credentials through ordinary TCP.

Linux `SO_PEERCRED` returns process credentials for connected AF_UNIX sockets;
the Linux manual restricts the option to AF_UNIX stream sockets and socket
pairs. It is not available on this TCP listener. Adding a Unix-domain endpoint
could supply local identity on Linux, but it would be a new local transport
interface and would require explicit configuration, lifecycle and portability
decisions. [Linux `unix(7)`](https://man7.org/linux/man-pages/man7/unix.7.html)

## Options

| Option | Effect | Assessment |
| --- | --- | --- |
| Keep current TCP semantics | Any process able to connect to the configured Fang listener can consume its bounded slots; active loopback saturation can deny peers | Preserves transparent TCP, compatibility and current trust boundary; limitation remains explicit |
| Scan `/proc/net/tcp` per accept | Infer socket owners from Linux process tables | Stage43 deferred this: Linux-only, O(host sockets) on the flood path, races with close, parsing and permission failure modes |
| Add a local AF_UNIX listener | Obtain peer credentials for local clients and optionally account by UID | Requires a new listener/configuration/API path, stream abstraction and non-Linux behavior; does not identify clients connecting through existing TCP |
| Add an application credential handshake | Identify callers independently of source address | Changes the current transparent byte-stream contract and requires protocol, credential lifecycle and compatibility design |
| Rate-limit loopback accepts or use one shared loopback quota | Bound bursts or reduce total local occupancy | Does not stop a client from retaining admitted active connections; a shared quota still cannot distinguish users |

## Recommendation and gate

Keep the existing TCP listener behavior and document the active-loopback
denial as an accepted local deployment limitation. The current interface has
no trustworthy per-user identity to drive such a quota, while the two ways to
add one (a new AF_UNIX endpoint or an application handshake) change the local
interface and compatibility contract. Do not add `/proc` scans, global
cross-Fang eviction, active-session preemption, a new wire protocol or an
operator quota as an implicit fairness fix.

If the owner wants local per-user fairness, first authorize a separate design
for an optional credential-bearing local endpoint, including supported
platforms, migration/compatibility behavior, whether existing TCP remains
available, and the UID-to-quota policy. Only then scope M2 implementation and
tests. Until that decision, Stage43's active-loopback limitation remains
accurate and Stage55's same-Fang idle policy remains unchanged.

## Evidence boundary

This is a source/API feasibility review and reuses the historical Stage43
active-flood measurements. No new runtime build, test, flood run, dependency,
configuration, wire-format or release-artifact change is claimed. In
particular, this survey does not assert a new reproduction on the Stage55
merged head.
