# Stage32 F5: QUIC initiator waits for handshake confirmation

Owner decision, 2026-10-28: the QUIC initiator waits until the receiver
has confirmed the handshake before it opens its stream. This fixes the
S3-P5 seed 5030 QUIC-hop reset (about 7.2 s, nothing echoed).

## The defect

Quinn (quinn-proto 0.11.15) completes the initiator's handshake when the
initiator sends its TLS Finished, and the initiator then sent its OPEN at
once, in a separate 1-RTT packet. On the receiver, Quinn discards 1-RTT
packets that arrive while its own handshake is still running
(`connection/mod.rs:2653`), without acknowledging them. RFC 9001 §5.7
allows a server to buffer them instead; Quinn does not.

On the S3-P5 relayed path (200 ± 100 ms jitter) the jitter put the OPEN
ahead of the Finished. The initiator had nothing else ack-eliciting to
send, so only its PTO could recover the OPEN, and with a smoothed RTT of
1.44 s that came too late: the relay's 5 s window for the initiator's
stream (`transport/quic.rs`, `accept_bi` deadline) closed first, the
relay closed the connection and the initiator reset its client.

Evidence, from an instrumented throwaway build (not certification),
results `20261028T204055Z-network`, run 3 (reset at 7.29 s; the
instrumentation patch and capture wrapper are in its `diagnostics/`):

- A: connection established +1.497 s, OPEN written +1.497 s, setup
  failed +7.346 s ("connection lost"); frames sent: STREAM 1.
- Y: connection established +1.982 s, `accept_bi` window closed
  +6.984 s; frames received: STREAM 0, ACK 8, CRYPTO 4.
- A capture on R shows every A→Y datagram forwarded: no loss on the
  wire (P5 has none).

The netem draws are seed-deterministic, so the same reordering recurred:
4 of 5 seed-5030 runs reset. Fail-closed, before authentication; under
spec §4.2 a clean early end, so no campaign verdict depended on it.

## The fix

`handshake_confirmed` (`crates/werewolfd/src/quic_fang.rs`) waits until
the connection has received HANDSHAKE_DONE, which the receiver sends
only after its own handshake completed. Quinn has no event for it, so
it polls the connection's frame counter every 5 ms and fails as soon as
the connection closes. It runs inside the existing client window
(`hs::CLIENT_WINDOW`), before the stream is opened, in:

- `connect_v3_on`: direct QUIC Fangs, the QUIC leg of failover and
  routed QUIC hops;
- the discovery RPC over QUIC (`discovery.rs`, `rpc_quic`), which had
  the same pattern.

Initiator timing only: every message, transcript, binding, deadline,
the receiver and trust are unchanged.

Cost: about one round trip more per QUIC session setup (the Finished
reaches the receiver, its HANDSHAKE_DONE comes back). If HANDSHAKE_DONE
is lost, Quinn retransmits it; the overall bound is still the client
window.

## Validation

- Test (`quic_fang/tests.rs`,
  `open_waits_for_receiver_to_confirm_the_handshake`): a UDP relay holds
  back the initiator's Finished for 300 ms so its 1-RTT packets overtake
  it. The initiator must send the OPEN exactly once. Without the fix it
  failed 5 of 5 runs (OPEN sent twice: discarded, then retransmitted);
  with it, it passed 5 of 5. Workspace tests, clippy `-D warnings` and
  fmt pass.
- Sandbox, diagnostic (not certification): S3-P5 on this change plus
  the harness of #84 (calibrated 2 MiB, F3 rule), results
  `20261028T225412Z-network`, `werewolfd` `6333a482…042c`. Seed 5030's
  QUIC hop completed for the first time, 118.95 s, hashes match (first
  echo 8.23 s); before, it reset at about 7.2 s in 4 of 5 runs. 5010
  completed (QUIC 115.3 s, TCP 107.7 s); 5020's QUIC hop reached the
  bound still progressing (1.97 MB echoed, last echo at 119.69 s, in
  order), EXCLUDED under F3. No reset, no mismatch; daemons alive after
  every run. First echoes on the QUIC hop, 8.2–10.1 s, are within the
  range seen before the change (8.2–10.3 s). An earlier run on this
  change used an uncalibrated 8 MiB payload (results
  `20261028T223044Z-network`; invalid workload, every leg at the bound);
  there too, seed 5030's QUIC hop set up and progressed.

Certification evidence is the Stage32 network campaign, run on a build
with this change when the owner resumes it.
