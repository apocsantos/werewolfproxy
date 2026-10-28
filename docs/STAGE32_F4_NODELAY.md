# Stage32 F4: TCP_NODELAY on encrypted-TCP sockets

Owner decision, 2026-10-28: set `TCP_NODELAY` on encrypted-TCP session
sockets, as a separate improvement. It is not the cause of the S3-P5 seed
5030 QUIC-hop reset (see "Not fixed" below).

## The defect

No Werewolf socket set `TCP_NODELAY`. Handshake messages are small
writes, and under Nagle a small write waits for the ACK of the previous
unacknowledged one. On the S3-P5 relayed path (200 ± 100 ms jitter on the
A–R and Y–R links) a capture on R showed the receiver's TLS server flight
leaving as two segments, 127 bytes and then 366 bytes, the second sent
less than 1 ms after the ACK for the first arrived: 0.44–0.73 s later, in
every leg. That time comes out of the receiver's 5 s setup window
(`hs::READ_WINDOW`, TCP accept to OPEN read), where the OPEN arrived
0.45–1.1 s before the window closed. Evidence:
`target/stage32-chaos/results/20261028T194101Z-network/cells/S3-P5-work/`
(`r.pcap`, `clients.log`; diagnostic run, not certification).

## The fix

`configure_session_socket` (`crates/werewolfd/src/transport/tcp_encrypted.rs`)
sets `TCP_NODELAY` and then the existing liveness settings
(`configure_liveness`, unchanged). It is used for the receiver's accepted
sockets and for the initiator's socket (`open_tcp`), so direct
encrypted-TCP Fangs, the encrypted-TCP leg of failover and routed TCP
outer hops all get it.

Socket options only: every handshake message, transcript, deadline, the
receiver's 5 s window and trust are unchanged. Unchanged paths: plain
target connections, including a relay's connection to the destination's
listener, and QUIC.

Cost: the kernel no longer coalesces small writes on these sockets, so
an application that writes in small pieces can produce more small
packets on the session's data path.

## Validation

- Tests (`tcp_encrypted/liveness_tests.rs`), each checked to fail with
  `set_nodelay` removed: the session-socket setup disables Nagle and keeps
  keep-alive and `TCP_USER_TIMEOUT`; the initiator's socket from
  `open_tcp` disables Nagle. Workspace tests, clippy `-D warnings` and fmt
  pass.
- Sandbox, diagnostic (not certification): S3-P5 with a capture on R, on
  a build with this change. The receiver's TLS flight left as one burst
  (127, 6 and 360 bytes in the same millisecond) in all five legs that
  reached B; the OPEN arrived 3.29–4.48 s after accept (before:
  3.86–4.55 s). Evidence:
  `target/stage32-chaos/results/20261028T201021Z-network/cells/S3-P5-work/`.

## Not fixed: S3-P5 seed 5030

The same validation run reproduced the seed 5030 QUIC-hop reset (7.18 s,
nothing echoed), with this change in place. The capture shows Y never
connected to B: the A–Y QUIC handshake completed, A sent what is most
likely the OPEN and received ACKs, and Y closed the connection about 5 s
after its handshake, matching Y's own window (`transport/quic.rs`,
accept-stream deadline or OPEN read). The cause is undiagnosed; it is
followed up separately.

Certification evidence is the Stage32 network campaign, rerun on a build
with this change when the owner resumes it.
