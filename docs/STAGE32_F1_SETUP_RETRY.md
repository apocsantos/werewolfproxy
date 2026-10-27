# Stage32 F1: encrypted-TCP setup retry

Owner decision, 2026-10-27: F1 is a robustness defect; fix it with one
TCP/TLS-only retry from the initiator, leaving the receiver's 5 s defence
unchanged. Retry boundary: up to the first byte of the receiver's
challenge. Scope: the direct encrypted-TCP Fang and the encrypted-TCP leg
of failover.

## The defect

Under 10% loss (Stage32 S1-P2, seed 2030) an encrypted-TCP Fang session
failed to establish where raw TCP stalled and recovered. A's SYN was lost,
then A's TLS ChangeCipherSpec; its retransmission came after the 3 s
minimum retransmission timeout that follows a lost SYN. A's TLS had
completed (under TLS 1.3 the initiator finishes first), but B's had not,
and B closed the connection 5.0 s after accepting it: the receiver bounds
TLS accept, the challenge and the OPEN read by `hs::READ_WINDOW` (5 s)
from TCP accept, against slow-handshake resource holding. Diagnosis and
captures: `docs/STAGE32_ROBUSTNESS_SPEC.md`, Step 3 notes.

## The fix

`connect_tcp_session` (`crates/werewolfd/src/transport/tcp_encrypted.rs`)
opens TCP, pinned TLS and runs the client handshake. If the attempt fails
before any byte of the receiver's challenge arrived, it makes exactly one
more, fully independent attempt: new TCP connection, new TLS session, new
challenge.

| Failure | Retried |
|---|---|
| TCP connect (refused, unreachable, 5 s timeout) | yes |
| TLS at the transport level (reset, close, 5 s timeout) | yes |
| TLS error: an alert, or a certificate that fails the selected-peer pin | **no** |
| Receiver closes, or the 5 s challenge read ends, with no challenge byte | yes |
| Any byte of the challenge arrived (then a bad, short or late challenge, a lost OPEN, a bad ACK) | **no** |
| The retry itself fails | **no** (one retry only) |
| No selected-peer key (fails before any socket) | **no** |

Why this is not a protocol change: until the challenge arrives the
initiator has sent no Werewolf handshake message, so the receiver holds
no OPEN, no lease and no target connection from it; the local application
socket has not been read. Every handshake message, transcript, window
and the receiver are unchanged. Unchanged paths: routed Fangs' outer
relay hop and discovery RPCs (`connect_authenticated_tcp` +
`client_handshake`, as before).

Cost: a receiver that refuses before speaking (Silver, admission full,
not listening) sees at most two connection attempts per local
connection instead of one. Worst case before the local connection is
reset: two attempts of up to 5 s TCP + 5 s TLS + 5 s challenge each.
Failover keeps its 15 s overall bound for trying providers.

## Remaining limit

The receiver's 5 s window also covers the OPEN read. If the challenge
arrived and A's OPEN is then lost long enough, the session ends and is
not retried (outside the owner's boundary).

## Validation

- Tests (`tcp_encrypted/setup_retry_tests.rs`), each checked to fail with
  the retry disabled or widened: a TCP close and a close before the
  challenge are retried once and the session then carries data exactly;
  never a second retry; a started challenge is not retried; a TLS pinning
  failure is not retried. Workspace tests, clippy `-D warnings` and fmt
  pass.
- Sandbox, diagnostic (not certification): 200 sequential 1 KiB sessions
  under P2 (seed 2030), three batches per build. Before: 6 of 600 failed
  (3 first sessions at about 3.05 s with no TCP packet captured; 3 when
  B's window expired before the challenge). After: 1 of 600 failed; B's
  window expired 4 times, the 3 before the challenge were each retried
  once and completed, and the 1 after the challenge (A's OPEN lost) was
  not retried, as designed. Under P2 with seeds 2010, 2020, 2030 the
  32 MiB S1 transfer completed with matching hashes (49–61 s). Evidence:
  `target/stage32-chaos/results/20261027T165530Z-network/diagnostics/f1-fix-validation/`.

Certification evidence is the Stage32 network campaign, rerun on a build
with this change when the owner resumes it.
