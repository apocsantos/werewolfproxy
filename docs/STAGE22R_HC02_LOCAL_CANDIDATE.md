# Stage22R HC02: encrypted TCP full-duplex WAN certification

Base: `efdaf71a89e160cd9c5a8d92b84b44974c11f54b` (LC01).
Branch: `codex/hc02-tcp-full-duplex-cancellation-safety`.
Pre-certification HEAD: `da0a4e8b372b0c75374239ba82b22df7cbe84e61`.
Status: WAN certified against the original RC1 VPS receiver. The candidate
`werewolfd` SHA-256 was
`8dad5819d99fac16b5519092e8f38da0d3aa8d3c681aa024925706d8c43e13fc`;
the remote original RC1 `werewolfd` SHA-256 was
`ea9ab16396a97263aadf88153a902fdd17c4aee1254246afb8a0add812d13ac4`.

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

## WAN acceptance evidence

The candidate ran on DarkSide against the original RC1 binary on the VPS.
The WAN validation reported:

- Short encrypted TCP echo: PASS.
- 64 MiB simultaneous/full-duplex echo: PASS. TX = 67,108,864 bytes;
  RX = 67,108,864 bytes; exact SHA-256 equality; zero TX/RX errors.
  The receiver target independently observed RX = 67,108,864 and
  TX = 67,108,864 bytes.
- 1 GiB simultaneous/full-duplex echo: PASS. TX = 1,073,741,824 bytes;
  RX = 1,073,741,824 bytes; both SHA-256 digests =
  `250e279ed22ae67955eb370b42b3fa1187333462e6e73f75da3fd79e0713c36d`;
  zero TX/RX errors. The receiver target independently observed
  RX = 1,073,741,824 and TX = 1,073,741,824 bytes, then clean EOF.
- HC01 local half-close with delayed reverse response: PASS.
- Remote-first EOF while the local write side remained open: PASS;
  complete response and EOF in 0.283279 seconds.
- Silver active-session authority fence: PASS. Before: active = 1,
  epoch = 1, locked = false. After: active = 0, epoch = 2,
  locked = true. The stale session received EOF and no
  post-linearization echo.
- Pack revoke active-session fence: PASS. Active sessions 1 → 0;
  packmates 1 → 0; global epoch unchanged. The stale session received EOF.
- New session while revoked: ConnectionReset, expected PASS.
- Same-Pelt re-add followed by a fresh session: PASS.

The WAN abort reproduced on LC01 at approximately 7 MiB did not recur with
HC02, including the successful 1 GiB full-duplex run. No wire/protocol or
security-model change was needed for backwards interoperability.

## Strict Clippy baseline comparison

The exact gate is
`cargo clippy --workspace --all-targets --all-features -- -D warnings`.
It fails at both the LC01 base and HC02 candidate on the same pre-existing
`clippy::needless_range_loop` at `werewolf-core/src/nodeid.rs:25`.
Because that error stops further crate analysis, both trees were then checked
with `-A clippy::needless_range_loop` added to expose the remaining failures.
Both produced precisely the same four `dead_code` diagnostics:

- `werewolfd/src/target_policy.rs:75`: `load` is unused;
- `werewolfd/src/authority.rs:29`: `PeerAuthority::Revoked` is unconstructed;
- `werewolfd/src/authority.rs:235`: `Authority::peer_state` is unused;
- `werewolfd/src/replay_v3_tests.rs:698`: `Daemon::control` is unused.

Finally, the full workspace/all-targets/all-features Clippy run passed on
**both** source trees with only `-A clippy::needless_range_loop -A dead_code`
added. Thus every HC02 strict-Clippy failure is baseline-equivalent; HC02
introduces no new Clippy failure. No warning is suppressed globally and no
unrelated baseline code was changed. The exact strict gate remains red on
both revisions.

Do not merge until the separate merge decision is made.
