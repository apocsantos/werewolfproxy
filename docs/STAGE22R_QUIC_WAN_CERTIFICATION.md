# Stage22R QUIC real-WAN certification

Source baseline: `main` at `6be8e5cd83f17b29ac1c0b1d5d1faa5eb02adb37`.
This document records operator-reported real-WAN validation. It changes no
production behavior, protocol, release artifact, or RC1 binary.

## Topology and interoperability

- DarkSide initiator across the public Internet to a remote Debian VPS receiver.
- DarkSide Fang profile: `wan-quic`; local listener: `127.0.0.1:17001`.
- VPS QUIC listener: `51.83.41.28:9560`; receiver-side target:
  `127.0.0.1:7000`.
- The VPS remained on the original RC1 receiver binary, providing backwards
  interoperability evidence. Its previously recorded `werewolfd` SHA-256 is
  `ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4`.

## Endpoint transition and short echo

Changing Pack `node-b` from TCP to QUIC invalidated the active TCP Fang under
the LC01 contract: `closed_fangs = 1` and `fang active` became empty. The
operator explicitly activated `wan-quic`; no active Fang was silently
live-refreshed. **PASS.** A subsequent short encrypted QUIC echo returned the
exact response. **PASS.**

## Sustained simultaneous full-duplex integrity

| Gate | Application TX | Application RX | TX/RX SHA-256 | Errors | Elapsed | Independent VPS target | Result |
|---|---:|---:|---|---|---:|---|---|
| 64 MiB | 67,108,864 | 67,108,864 | `e966427143da9f7e64951e8f8a9f435a540fcef5247558ed452594c71ffdd458` (identical) | TX `None`; RX `None` | 56.498 s | RX = 67,108,864; TX = 67,108,864; 56.382746 s | PASS |
| 1 GiB | 1,073,741,824 | 1,073,741,824 | `690ff27876c59efa232352bb235eaae0d7638abbdfaa7969cccca43b58be201c` (identical) | TX `None`; RX `None` | 995.670 s | RX = 1,073,741,824; TX = 1,073,741,824; 995.534048 s | PASS |

For the 1 GiB run, `HASH_MATCH = True` and `SIZE_MATCH = True`. The VPS
target independently counted every received and echoed byte. Both sustained
WAN transfers completed without a sender or receiver error.

## Stage11C authority fences over QUIC

### Silver

- Before `silver on`: active = 1, epoch = 3, locked = false. The established
  session successfully echoed before the fence.
- After `silver on`: active = 0, epoch = 4, locked = true,
  lifecycle = `LOCKED`. The stale session received RX EOF;
  `MATCH_AFTER = False`, with no post-linearization echo.
- A new session while Silver was active failed with `ConnectionResetError`.
- Silver reset advanced the epoch to 5, set locked = false, and returned the
  lifecycle to `READY`.

Silver authority fence: **PASS.**

### Per-peer Pack revoke and re-add

- Before revoking `node-a2`: active = 1, epoch = 5, packmates = 1. The
  established session successfully echoed before revocation.
- After revoke: active = 0, epoch = 5, packmates = 0, locked = false. The
  global epoch remained unchanged. The stale session received RX EOF and
  `MATCH_AFTER = False`.
- A new session while revoked failed with `ConnectionResetError`.
- Re-adding the same public Pelt restored packmates = 1;
  protected-state generation = 16. A fresh QUIC session returned the exact
  echo, `MATCH = True`.

Per-peer revoke, rejection while revoked, and same-Pelt recovery: **PASS.**

## Certification conclusion

Sustained QUIC integrity passed over the real WAN through 1 GiB, with matching
end-to-end hashes and independent receiver-target byte counts. Silver and
per-peer revoke fenced established QUIC sessions and rejected new sessions
while authority was absent. Together with the recorded TCP WAN results, this
demonstrates TCP/QUIC Stage11C authority parity on the tested path. LC01
endpoint invalidation remained effective, and QUIC forwarding interoperated
with the original RC1 VPS receiver. These claims are limited to the topology,
binary pairing, and runs recorded above.
