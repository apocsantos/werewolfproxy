# Session-end signalling — proposal for review

**Status: proposal, 2026-10-19. Nothing here is implemented.** It changes how
an abnormal session end is signalled between wolves, so it needs explicit
review before any code changes (see the crypto/protocol change discipline).

## Goal

An application behind a Fang must never see a failed transfer as a complete
one. PR #12 made Werewolf close the *local* application socket and the
*target* socket with a TCP reset (RST) on an abnormal end, and with FIN only
on a clean end. That is only correct if the signal **between the wolves** is
also correct: when one wolf sees a clean end, the other must have ended
cleanly. Three places still turn an abnormal end into a clean one.

| # | Where | Today | Effect |
|---|---|---|---|
| 1 | QUIC receiver, target fails after the ACK | `send.finish()` (clean FIN) | Initiator's app gets FIN after a truncated download |
| 2 | QUIC initiator, local side fails | `send` dropped → Quinn's `Drop` calls `finish()`; then `close(0)` | Receiver may give the target FIN after a truncated upload |
| 3 | Encrypted TCP, either direction | Any error while reading a frame's length prefix → treated as EOF | Peer crash, TCP reset, path cut or missing TLS `close_notify` all look clean |

A related race on the **clean** path (2b below) can do the opposite: report
a successful session as failed.

## Gap 1 — QUIC receiver finishes the stream on target error

`transport/quic.rs` (forwarding result, `lease.is_current()` branch): when
the target connection fails after the ACK, the receiver calls
`send.finish()` and `recv.stop(V3_REJECT_CODE)`. The comment explains why:
`reset()` could discard a queued, not-yet-acknowledged ACK.

The initiator's `copy(recv → local)` then ends with `Ok`, `proxy_streams`
succeeds, and the local application gets FIN.

**Proposal:** call `send.reset(V3_REJECT_CODE)` here, exactly as the
revocation/Silver branch next to it already does after the ACK.

- If the reset overtakes the ACK, the initiator sees a failed handshake: it
  closes the connection and its local socket with RST. If the ACK arrived
  first, its stream read fails and it resets the local socket. **The
  application sees RST either way**; only the initiator's log differs.
- No transport fallback exists per session (transport choice is the
  operator's / selector's, not automatic on rejection), so a handshake-looking
  failure cannot trigger a downgrade.
- Bytes the target sent before failing but the initiator had not received
  are lost. That matches TCP RST semantics: the transfer failed anyway.
- **Wire impact:** none new. `V3_REJECT_CODE` after an ACK is already sent
  on revocation and Silver, and the initiator already handles it.

**Alternative:** a new stream error code (for example `0x575704`, "session
aborted") so the initiator can log a target failure separately from a
rejection. Not recommended: a new wire constant for a log line, and it
tells an authenticated peer more about the receiver's target than today.

## Gap 2 — QUIC initiator implicit FIN, and the close race

`quic_fang.rs`: `proxy_streams` returns an error when the local application
resets, a local read/write fails, or the receiver's stream fails. Its
`SendStream` is then dropped. In Quinn 0.11.9, `Drop for SendStream` calls
`finish()` unless the connection has already failed, so a clean FIN is
queued. `CloseClientConnection` then calls `connection.close(0, …)`.
Whether the receiver sees the FIN (target gets FIN → truncated upload looks
complete) or the close (error → target RST) depends on packet timing.

**2a, abnormal path — proposal:** on error, call `send.reset(V3_REJECT_CODE)`
and `recv.stop(V3_REJECT_CODE)` explicitly before dropping them, so the
receiver always takes its error path (target RST).

**2b, clean path:** after a successful `proxy_streams`, the connection is
closed at once with `close(0)`. Quinn's `close` abandons unacknowledged
stream data, so the receiver can lose the tail of the upload and the FIN and
then reset the target, reporting a clean session as failed. Two options:

- **(a) Wait for acknowledgement.** After `finish()`, wait for
  `send.stopped()` before closing, bounded by a short timeout. Quinn yields
  `Ok(None)` once the peer's QUIC stack has acknowledged all stream data. Its
  documentation is explicit that this is *not* delivery to the application:
  after `close()` the peer may still drop acknowledged but unread data. This
  narrows the race to the receiver's own read latency; it does not remove it.
  Adds about one RTT per Fang connection. Initiator-only change.
- **(b) Let the receiver end the connection.** After a stream completes
  cleanly in both directions, the receiver closes the connection (code 0) and
  the initiator waits, bounded, for `connection.closed()` instead of closing
  first. The side that last reads the data decides the end, so no read can be
  cut short. It changes the receiver's connection lifecycle (today it keeps a
  connection open for further OPEN streams until the initiator closes), and an
  older receiver never closes, so the initiator still needs the bounded
  fallback close.

Recommended: (a) now, plus test 2 below run many times; move to (b) only if
the test still shows a clean upload reported as reset.

**Wire impact:** none for 2a and (a); (b) changes who sends
CONNECTION_CLOSE but uses existing frames and codes.

## Gap 3 — encrypted TCP treats any cut as a clean end

`transport/tcp_encrypted.rs`, `read_encrypted_frame`:

```rust
if reader.read_exact(&mut len_buf).await.is_err() {
    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed"));
}
```

Both callers (client and server pipes) treat `UnexpectedEof` as a clean end
and send FIN to the application or target. So all of these look clean:

- the peer sent TLS `close_notify` (the real clean end);
- the peer's TCP closed **without** `close_notify` (crash, killed process,
  dropped stream) — rustls reports this as `UnexpectedEof`;
- a TCP reset (`ECONNRESET`) or kernel liveness timeout;
- a cut inside the 4-byte length prefix.

(A cut inside a frame body already fails `read_exact` on the ciphertext and
is an error; a forged or truncated frame fails AEAD.)

**Proposal:** a frame boundary is the only place a clean end may occur, and
only after `close_notify`:

1. Read the first length byte with `read()`. `Ok(0)` there means rustls
   received `close_notify`: return "clean end" (for example `Ok(None)`).
2. Every other outcome — rustls `UnexpectedEof` (no `close_notify`), any I/O
   error, EOF after a partial prefix — is an error, and the caller takes the
   abnormal path (RST to application or target).

The sender side needs no change: a clean direction end already calls
`shutdown()` on the TLS writer, which sends `close_notify` before TCP FIN.
TLS 1.3 `close_notify` closes only that direction, so half-close keeps
working.

**Wire impact:** no new bytes; the receiver now *requires* the existing
`close_notify` for a clean end. **Compatibility risk:** any peer (including
the RC1 binary) that ends a clean session without `close_notify` would now
cause an RST. The server pipe ignores `shutdown()` errors (`let _ =`), so
this must be checked by test, not assumed.

**Alternative:** keep accepting a bare TCP FIN at a frame boundary and only
treat resets, liveness timeouts and partial prefixes as abnormal. Smaller
compatibility risk, but a crashed peer whose kernel sends FIN still looks
clean — the main case this gap is about. Not recommended unless the RC1
interop test shows RC1 omits `close_notify`.

## Scope and non-goals

- `tcp-plain` has no integrity or close authentication and is out of scope.
- No change to handshakes, transcripts, keys, nonces, replay rules, trust,
  Pack/Pelt semantics or transport selection.
- RPC (Stage24) results already end with `finish()` / `close_notify` after a
  complete, signed result; unchanged.

## Test plan

For each gap, a test that fails today and passes after the change:

1. QUIC receiver: target resets mid-download → initiator's local client gets
   `ConnectionResetError`, never a clean EOF; also with the target failing
   immediately after the ACK (reset may overtake the ACK).
2. QUIC initiator: local client resets mid-upload → target sees RST, no clean
   EOF, across repeated runs; and a clean upload followed by immediate close
   delivers every byte and a FIN to the target (2b).
3. Encrypted TCP: peer TCP closed without `close_notify`, TCP reset, and EOF
   after 1–3 prefix bytes each give RST to the application/target; a clean
   end with `close_notify` still gives FIN, including half-close.
4. Interop: the `replay_v3` legacy matrix with the RC1 binary on both sides —
   clean sessions must still end with FIN. This decides 3 vs its alternative.
5. Lab gates (`run.py`, `service_gate.py`, `discovery_gate.py`) unchanged and
   passing; the D05B/idle-liveness lab checks repeated for QUIC and TCP.

## Decisions requested

1. Gap 1: reset with the existing `V3_REJECT_CODE` (recommended), or a new
   "session aborted" code.
2. Gap 2: explicit reset on error (recommended); for the clean path, option
   (a) bounded `stopped()` wait (recommended first) or (b) receiver-initiated
   close.
3. Gap 3: require `close_notify` for a clean end (recommended, subject to the
   RC1 interop result), or accept bare FIN at a frame boundary.
4. Whether to implement all three in one PR (one review) or one PR per gap.
