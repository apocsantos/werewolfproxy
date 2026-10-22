# Stage22R-D08: real-WAN resource pressure certification

Source baseline: `main` at `bcf7bbd` (`werewolfd`
`c17d76f9327317b8333f2fcf9e57cb5f68ad7e1573ceb1be93ce575f2eb161f8`,
`werewolfctl`
`fceaccbfe4f39edcb0531a8e1558e8d9f0fa4d06d06be81518cadd99477bc491`), deployed
on both nodes for this run. Runs executed on 2026-10-22 between 18:21 and
19:49 (`+01:00`). This document records real-WAN evidence from the DarkSide
initiator and from a read-only sampler on the Debian VPS receiver. No
production code, protocol, release artifact, firewall setting or network
interface configuration changed here. Historical local wolves (PIDs 1270
`wolf-a`, 1273 `wolf-b`) and the VPS echo target (PID 4481) were untouched.

## Purpose and method

[Stage16](STAGE16_PREAUTH_RESOURCE_HARDENING.md) certified finite admission
ceilings, but on loopback with test-sized limits; [Stage20](STAGE20_ADVERSARIAL_SOAK.md)
soaked on loopback. D08 asks whether the **production** ceilings hold on the
deployed receiver (`werewolfd.service`, `LimitNOFILE=4096`) under pressure
sent across the real WAN: excess work is refused promptly without a queue,
an established session is undisturbed, every resource returns to its
baseline, and the daemon recovers without a restart.

All pressure came from DarkSide (one source address) to the VPS
`51.83.41.28` (`:8443` TCP, `:9560` QUIC) in short single bursts with
cool-downs between cases, far below flood rates.

Throughout the run a **witness** session held one Fang session through the
`wan-quic` Fang, sending 64 KiB/s of deterministic data and checking every
echoed 4 KiB block; any mismatch, reset or EOF would fail the whole run.

Disposable test infrastructure outside the repository:

| File | SHA-256 |
|---|---|
| `sampler.py` (VPS, read-only: `systemctl show`, `/proc`, `werewolfctl --json status`, journal line count, 1 s) | `37c0a89f4d24158cd5642ff0d0e546b9189899a3b7de30ffe2732618c77fd689` |
| `/tmp/stage22r-d08/witness.py` | `45df9012f42cdc99d8aac896353349c6a17766d0a0be031196c979f0156861f6` |
| `/tmp/stage22r-d08/sessions.py` (P1) | `6dd2fe1c41eabefbbe09e78fbc392a324cde9725ab1f4717022e1681dd904baa` |
| `/tmp/stage22r-d08/tcpstall.py` (P2) | `5163c082db60574d607c4c8e05ee88d9135649acd9f5f517d4025da54fdb1014` |
| `/tmp/stage22r-d08/quicstall` (P3, Rust; `src/main.rs` `0681f018…d58e`) | `193f463fd54f6509f1d1ccf135579fba3a7fca8f8879a0ae760eda02353158d0` |
| `/tmp/stage22r-d08/slowtarget.py` (P5) | `26ebd812301cca53e071c95949ac76c0d8204c8a15c9f05bf2bc855c22bca7e4` |
| VPS samples (5,564 lines, 0 sampler errors) | `3c9d0d8cf5e95b39e3f932880f6bb1e68edcdbbaa8969418136d4932e3889c27` |

`quicstall` completes the QUIC/TLS handshake (the receiver requests no
client certificate) and never opens a stream; it does not verify the server
certificate because it never sends application data. Both stall tools were
first checked against the local Stage22R daemon on loopback: the receiver
closed every stalled connection at 5.0 s.

## Setup

| Item | Node | State |
|---|---|---|
| Upgrade to `bcf7bbd` | DarkSide | Stage22R daemon restarted by exact PID; new PID 1875952, exe `c17d76f9…`; `READY`, generation 67; `wan-quic` restored automatically |
| Upgrade to `bcf7bbd` | VPS | INSTALL.md steps, backup in `/root/werewolf-backup/20261022-bcf7bbd/`; `sha256sum -c` OK for both binaries and the sampler; installed and running exe (`/proc/499679/exe`) `c17d76f9…`, `werewolfctl` `fceaccbf…`; `MainPID` 499679, `NRestarts=0`; open files limit 4096 (soft and hard) |
| `target allow node-a2 192.0.2.1:7000` (P5 only) | VPS | `192.0.2.1` (TEST-NET-1) verified to hang from the VPS (`exit 124` after 8 s); removed after P5, list back to `127.0.0.1:7000` only |
| One-off Fang `127.0.0.1:17301` → `192.0.2.1:7000` (P5 only) | DarkSide | Closed after P5 |

## Cases and results

| Case | Pressure | Expected ceiling | Observed | Verdict |
|---|---|---|---|---|
| P1 session cap | Sessions opened one at a time through `wan-quic`, each proven by an echoed probe, until 3 consecutive opens failed; all held sessions re-probed; all closed; 15 s pause; 3 rounds | 64 authority sessions per peer | Each round: 63 held + witness; VPS `inbound_authority.active` peaked at exactly **64**; each excess open reset after 0.09–0.10 s; 0 of 63 held sessions failed the re-probe; after close, `active` back to 1 (witness) within 1 s and the next round reached 64 again | PASS |
| P2 stalled TCP | 300 TCP connections to `:8443` at once, each sending one byte (`0x16`) then silent | 256 shared handshake permits, 5 s TLS deadline | 300 connected; **256** held and closed by the receiver (EOF) at 5.046–5.114 s; **44** reset at 0.077–0.364 s; VPS descriptors 17 → 273 → 17 | PASS |
| P3 stalled QUIC | 200 QUIC connections at once, each completing the handshake and never opening a stream | 128 QUIC connection contexts, 5 s first-stream deadline | **127** completed (128 less the witness's connection) and were closed by the receiver at 5.049–5.498 s; **73** were refused explicitly ("the server refused to accept a new connection") within 0.141–1.057 s | PASS |
| P4 RPC rate limit | Two concurrent waves of 16 `discover services` to the VPS, back to back | 16-request bucket per peer, refilling 4/s | Wave A (0.54 s): 16 `ok`; wave B (0.41 s later): 2 `ok`, 14 `rejected`; one query 4 s later: `ok` | PASS |
| P5 slow targets | 12 sessions at once through a Fang to the granted, unanswering `192.0.2.1:7000` | 8 DNS/connect permits per peer, 5 s connect deadline | VPS `establishing` peaked at **8**; those 8 sessions reset at 5.117–5.269 s; the other **4** reset at 0.267–0.270 s | PASS |

Two earlier P4 attempts are **excluded**, not counted. The first sent 40
queries sequentially at about 3.7/s, below the refill rate, so all 40 were
`ok` and the limit was never reached. The second sent 40 concurrently and
DarkSide's own control socket admitted only 16 clients (the others ended
with `Broken pipe` or `Connection reset`, exit class 69) before anything
reached the VPS; that shows the local control limit, not the receiver's.
The two-wave run above replaced them.

### Legitimate traffic during saturation (recorded, not judged)

Stage16 makes no claim of reserved capacity: the TCP and QUIC handshake
permits are shared, and the DNS/connect permits are shared per peer. A new
legitimate session started during each burst was therefore expected to be
refused, and was:

| During | New session through a Fang | After the burst |
|---|---|---|
| P2 (1 s and 3 s in) | No echo (2.01 s, 0.05 s) | Echo in 0.15 s, 1 s after the last stalled connection closed |
| P3 (1 s and 3 s in) | No echo (0.06 s, 0.05 s) | Echo in 0.14 s |
| P5 (1 s in), to `127.0.0.1:7000` | Reset after 0.10 s | Echo immediately |

The established witness session was unaffected by every burst.

## Witness

| Start | End | Sent = received | SHA-256 (both sides) | Errors |
|---|---|---|---|---|
| 18:23:47 | 19:49:06 | 335,470,592 bytes | `64e23e649092bea3f8674224c764b196ae96ee550a5d88faf1a43c5cbfa40948` | none; never more than one 4 KiB block in flight |

## Receiver stability

From 5,564 one-second VPS samples (17:18:59–18:52:22 UTC), covering the
upgrade settle, the baseline, every case and the tail:

- `MainPID` 499679 throughout, `NRestarts=0`, `ActiveState=active`.
- `lifecycle` always `READY`; `storage_degraded` always false;
  `inbound_authority` always healthy and never locked.
- **Journal: 0 lines** from the daemon for the whole sampled period: no
  line per refused connection, reset, stalled handshake or rate-limited
  request (Stage16's bounded-logging claim).
- **Threads: 3** throughout (4 briefly after the run).

| Phase | Descriptors | RSS (KiB) | `active` / `establishing` |
|---|---|---|---|
| Idle baseline | 16 | 8,724 | 0 / 0 |
| Witness only | 17 | 8,820 | 1 / 0 |
| P1 peak | 80 | 12,360–12,488 | 64 / 0 |
| 30 s after P1 | 17 | 12,488 | 1 / 0 |
| P2 peak | 273 | 13,780 | 1 / 0 |
| 30 s after P2 | 17 | 12,916 | 1 / 0 |
| 55 min idle between P2 and P3 | 17–18 | 12,916–12,924 | 1 / 0 |
| P3 peak | 17 | 14,296 | 1 / 0 |
| P4 (all three attempts) | 17 | 14,272 flat | 1 / 0 |
| P5 peak | 25 | 14,428 | 1 / 8 |
| Tail and after the witness | 17, then 16 | 14,428–14,448 | 1, then 0 / 0 |

Descriptors return to the baseline within a second of each case ending. RSS
rises once per new kind of peak load and then stays flat; repeating a load
adds almost nothing (P1 rounds: 12,360 → 12,436 → 12,484 KiB at each
64-session peak) and 55 idle minutes added 8 KiB. This is the allocator
keeping its high-water mark, as explained in the
[D07 RSS follow-up](STAGE22R_D07_TARGET_FAILURE_CERTIFICATION.md), not a
per-session leak. The peak was 14,448 KiB.

## Claims and nonclaims

| Claim | Result |
|---|---|
| D08-1: the per-peer authority ceiling (64) holds over the WAN, refuses excess at once, and releases fully on close, repeatably | CERTIFIED (P1) |
| D08-2: stalled pre-auth TCP is capped at 256, refused beyond without a queue, and released at the TLS deadline | CERTIFIED (P2) |
| D08-3: stalled QUIC connections are capped at 128 contexts, refused explicitly beyond, and released at the first-stream deadline | CERTIFIED (P3) |
| D08-4: the discovery RPC rate limit holds per peer and refills | CERTIFIED (P4) |
| D08-5: per-peer DNS/connect work is capped at 8 with the 5 s deadline, and excess is refused at once | CERTIFIED (P5) |
| D08-6: an established session is undisturbed by every burst | CERTIFIED (witness) |
| D08-7: the receiver recovers without restart; descriptors return to baseline; memory plateaus; logging stays bounded | CERTIFIED |

D08 does not claim:

- Capacity for new legitimate sessions while the shared permits are
  saturated (Stage16 nonclaim; observed refused, see above).
- Resistance to distributed or continuous refilling floods, bandwidth or
  CPU exhaustion.
- The 1,024 global authority ceiling or the descriptor limit: one peer is
  capped at 64 sessions, so reaching them needs many Pack identities.
- Encrypted-TCP Fang sessions for P1/P5: the VPS Pack address is
  `quic://…:9560`; P2 exercised the TCP listener's pre-auth path only.
- Which of two equal ceilings refused P1's 65th session: the 64 authority
  sessions per peer and the 64 QUIC streams per connection coincide; the
  VPS `active` count reached exactly 64.

STAGE22R_D08_RESOURCE_PRESSURE = CERTIFIED on `bcf7bbd`
