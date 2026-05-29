# WerewolfProxy Status

## Current Version
v0.3-prealpha

## Working Components

- Pelt: persistent Ed25519 identity
- Pack: persistent trusted peers
- Fang: local reverse tunnel
- Fang Session v1: signed HELLO and signed ACK
- Secure Fang v1: ephemeral X25519 + ChaCha20-Poly1305 encrypted wolf-to-wolf leg
- Silver v1: lockdown mode, rejects new Fang opens
- Multi-Den local lab: two wolves on one machine

## Test Topology

Wolf A:
- socket: /tmp/wolf-a.sock
- home: ~/.config/wolf-a
- listen: 127.0.0.1:8443

Wolf B:
- socket: /tmp/wolf-b.sock
- home: ~/.config/wolf-b
- listen: 127.0.0.1:9443

Private service:
- 127.0.0.1:8080

Client tunnel:
- 127.0.0.1:9003

## Verified

curl http://127.0.0.1:9003 successfully reaches service on 127.0.0.1:8080 through Secure Fang.

## Next Steps

1. Fang task cancellation
2. Hide v1 packet padding
3. Config cleanup
4. Proper QUIC transport
5. Qt desktop UI
6. Moon rendezvous layer
