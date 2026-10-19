# WerewolfProxy Transport Architecture

The normative rules are frozen in the [v1 wire contract](docs/V1_WIRE_CONTRACT.md)
and [v1 CLI contract](docs/V1_CLI_CONTRACT.md). This page is an overview; where
it differs from those contracts, the contracts win.

## Transports

`werewolfd` implements three Fang transports (`crates/werewolfd/src/transport/`):

| Transport | Module | Wire generation | Security |
|---|---|---|---|
| QUIC | `quic.rs` | fang-quic-v3 | TLS 1.3, exact-SPKI Pelt binding, OPEN bound to TLS exporter |
| Encrypted TCP | `tcp_encrypted.rs` | fang-tcp-v3 | TLS 1.3, exact-SPKI Pelt binding, challenge + X25519, AEAD frames |
| Plain TCP | `tcp_plain.rs` | none | Unauthenticated and unencrypted; explicit opt-in only |

A Fang profile picks one transport with
`fang create ... --transport quic|tcp|tcp-plain`. That argument selects the
transport for that profile; it is not a fallback chain.

## Selection and fallback

The local automatic selector defaults to `secure` (an alias for `strict`):

```text
QUIC secure → encrypted TCP secure → fail
```

It never silently selects plain TCP. `compatibility` uses the same secure order
unless the invocation explicitly passes `allow-plain-fallback`. `LEGACY`
deliberately keeps the historical weaker fallback behavior. These are local
choices, never a remote capability or downgrade negotiation.

## Endpoints

Each Pack entry has exactly one address, and Pack validation rejects two
entries with the same fingerprint or public key, so one identity cannot be
added twice as transport-specific aliases.

A Fang requesting `tcp` or `tcp-plain` reuses the peer's address with the
scheme rewritten to `tcp://` and the same host and port (`policy.rs`). A
receiver reachable over both transports from one Pack entry therefore listens
on the same port number for TCP and UDP, for example
`--listen 0.0.0.0:9560 --quic-listen 0.0.0.0:9560`. When the listeners use
different ports, switching transport means changing the address with
`pack set-address`, as in the Stage22R tests.

Changing a Pack member's address invalidates its active Fangs; they are not
live-refreshed onto the new endpoint (Stage22R LC-01).

## Idle sessions and liveness

An idle Fang session stays open while the path and peer are alive:

- **QUIC:** the initiator sends a keep-alive PING after 10 s of silence
  (`quic_fang.rs`). The 30 s idle timeout still applies, so a dead path
  ends the session about 30 s after the last packet was received.
- **Encrypted TCP:** there is no application idle timeout. Both ends enable
  kernel TCP keep-alive (first probe after 30 s idle, then every 10 s,
  3 probes) and a 60 s `TCP_USER_TIMEOUT` for unacknowledged data
  (`tcp_encrypted.rs`).

This supersedes the earlier behaviour, where idle sessions were closed after
about 30 s on QUIC (Stage16: no keep-alive) and 60 s on encrypted TCP (a
per-frame read timeout), which broke idle interactive sessions such as SSH.
Keep-alive PINGs and TCP probes are additional observable traffic; they do
not change any frame format or handshake.

## Abnormal session end

A Fang closes the local application's socket, and the receiver closes its
target socket, with a TCP reset (RST) when a session ends abnormally: a
transport error, a dead path, cancellation, revocation or Silver. Only a
clean end of both directions closes with FIN. An application therefore sees
a failed transfer as an error, not as an EOF that looks like completion.

Between the wolves, the QUIC receiver resets its stream (it no longer
finishes it) when the target fails after the ACK. Two cases still surface as
a clean EOF and are being fixed under the approved
[session-end signalling proposal](docs/SESSION_END_SIGNALLING_PROPOSAL.md): a
failing QUIC initiator's dropped stream sends an implicit FIN, and encrypted
TCP treats any cut at a frame's length prefix (missing TLS `close_notify`,
TCP reset, partial prefix) as a clean end.

## Responsibilities

Each transport covers the same lifecycle:

1. open the local listener
2. connect to the peer
3. authenticate the receiver (TLS 1.3 + Pelt SPKI) and the sender (signed OPEN)
4. request an authorized remote target
5. pipe bytes in both directions, including half-close
6. close gracefully on cancellation

## Not yet implemented

The code currently dispatches on a `FangTransport { Tcp, Quic }` enum and a
`plain_tcp` flag (`transport/mod.rs`). Planned but not built:

- a shared `FangTransport` trait with uniform
  `connect` / `open` / `pipe` / `health` / `shutdown` operations
- transport health scoring (availability, recent failures, latency,
  throughput, reconnect count, last success, last error)
- background recovery checks and automatic return to the preferred transport

## Validation

- Stage9–20 security and soak certification, frozen in the
  [Stage21 RC1 audit](docs/STAGE21_RC1_AUDIT_FREEZE.md).
- Stage22/22R real-WAN validation of QUIC and encrypted TCP: sustained
  full-duplex integrity, receiver/initiator restart, hard crash and hard path
  loss. See the [documentation index](docs/INDEX.md).
