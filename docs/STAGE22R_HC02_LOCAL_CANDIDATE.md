# Stage22R HC02: encrypted TCP full-duplex local candidate

Base: `efdaf71a89e160cd9c5a8d92b84b44974c11f54b` (LC01).
Branch: `codex/hc02-tcp-full-duplex-cancellation-safety`.
Status: local candidate. Two-host WAN acceptance has not been run; HC02 is not WAN certified.

## Cause and correction

The HC01 client forwarding loop rebuilt two compound futures on each
`tokio::select!` iteration. When one direction became ready, the other future
was dropped even if it had consumed plaintext, a frame prefix or ciphertext,
or had begun submitting an encrypted frame. The next iteration resumed from
the same socket position with a new frame parser or writer future. This could
lose application bytes or desynchronize the encrypted framing stream.

`secure_copy_client_side` now creates one future for each direction and
selects on them once. The request future owns its application read, AEAD
counter, encrypted writer and frame submission across polls. The response
future owns its encrypted reader, AEAD counter and application output across
polls. Local request EOF shuts down the encrypted writer and awaits the
response future. Remote response EOF shuts down local output and ends the
request future. A real error ends the session. Revocation, Silver and Fang
invalidation may still cancel a whole session; no cancelled frame is resumed.

No wire bytes, framing, padding, nonce, counter, handshake, identity, trust,
authorization, authority generation, persistence, CLI, QUIC or plain TCP
protocol rule changed.

## Local evidence

- Two controlled regressions pause after partial encrypted-frame read and
  write. Both failed on the LC01 baseline and pass with the correction.
- The production-path full-duplex test verifies exact content and counts for
  2 MiB while sender, echo target and receiver operate concurrently with
  small reads/writes and scheduling yields. A 256 KiB variant passed 100/100
  independent runs and checks receiver authority release each time.
- HC01's production-path test now holds the target response behind an explicit
  signal after receiving client request EOF. The response and final EOF pass.
  Existing remote-first EOF, short-lived authority, revoke and Silver tests
  remain in the workspace suite.
- `cargo fmt --check`: pass.
- `cargo test --workspace -- --test-threads=4`: pass, 203 Rust tests in total
  (169 werewolfd, 22 werewolf-core unit, 12 other core tests). A first default
  concurrency run had one transient `EAGAIN` while an existing replay test
  created its fixture; that test passed alone and in the four-thread rerun.
- Python integration discovery: 64/64 pass.
- `python3 tests/integration/run.py`: 12/12 pass, including 50 MiB encrypted
  TCP integrity and restart/profile restoration.
- `cargo build --workspace --locked`: pass. Local `target/debug/werewolfd`
  SHA-256: `8dad5819d99fac16b5519092e8f38da0d3aa8d3c681aa024925706d8c43e13fc`.

The exact strict command
`cargo clippy --workspace --all-targets --all-features -- -D warnings`
fails on five warnings already present at the LC01 base: one
`needless_range_loop` in `werewolf-core/src/nodeid.rs` and four dead-code
warnings in werewolfd. With only those two baseline warning categories
allowed, the full workspace Clippy run passes. No unrelated source was
changed to suppress these warnings. The exact strict gate remains open.

## WAN acceptance still required

Install this candidate only on DarkSide. Keep the VPS on the original RC1
daemon with SHA-256
`ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4`.
Record both binary hashes before traffic. Then run short echo, HC01 half-close,
remote-first EOF, 64 MiB and 1 GiB simultaneous full-duplex echo with SHA-256,
Silver active-session fencing, and revoke active-session fencing. The 1 GiB
gate must report exactly 1,073,741,824 bytes sent and received, matching
SHA-256 digests and no sender or receiver error. Do not merge before these
gates pass.
