# WerewolfProxy Transport Architecture

## Current transport stack

WerewolfProxy currently supports three operational Fang transports:

1. QUIC encrypted
2. TCP encrypted v2
3. TCP plain

Current preferred runtime order:

```text
QUIC → TCP encrypted v2 → TCP plain
Operational peer model

Because QUIC and TCP use different listener endpoints, the current local lab uses transport-specific Pack aliases:

wolf-a       → quic://127.0.0.1:9560
wolf-a-tcp   → tcp://127.0.0.1:8443

Both aliases may reference the same peer identity fingerprint and public key.

Transport responsibilities

Each transport should eventually expose the same conceptual operations:

open local listener
connect to peer
authenticate peer
request remote target
pipe bytes
report health
close gracefully
Desired internal model

Future transport code should move toward a common abstraction:

FangTransport
├── QuicTransport
├── TcpPlainTransport
└── TcpEncryptedV2Transport

Each transport should provide equivalent lifecycle behavior:

connect()
open()
pipe()
health()
shutdown()
Health model target

The transport manager should eventually score transports using:

availability
recent failures
latency
throughput
reconnect count
last successful request
last error
Auto-healing target

The transport manager should support:

primary transport selection
automatic fallback
background recovery checks
automatic return to preferred transport
stale Fang cleanup
persistent Fang restore
Current validated baseline

The current RC baseline validates:

QUIC tunnel
TCP encrypted v2 tunnel
TCP plain fallback tunnel
automatic fallback
QUIC restoration
50 MiB TCP encrypted v2 SHA256 integrity transfer
systemd restart recovery
local reinstall recovery
EOF
