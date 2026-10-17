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

## Endpoints and Pack aliases

QUIC and TCP listen on different endpoints, so a peer reachable over both is
added to the Pack as transport-specific aliases that share one identity:

```text
wolf-a       → quic://127.0.0.1:9560
wolf-a-tcp   → tcp://127.0.0.1:8443
```

Changing a Pack member's address invalidates its active Fangs; they are not
live-refreshed onto the new endpoint (Stage22R LC-01).

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
