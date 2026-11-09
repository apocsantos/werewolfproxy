# Stage41 M1 — QUIC initiator exposure, reproduced

**M1: CONFIRMED.** On `main`, a QUIC receiver can open streams toward the
initiator that the initiator never reads, and can send it datagrams. In
a measurement, the initiator accepted **249 MB** of unread data on one
connection within 8 s. That matches the survey's arithmetic bound
(~250 MB).

No production source changed. The M1 test is committed `#[ignore]`d, so
`cargo test` on `main` stays green. M2 removes the ignore.

## 1. Baseline

- `main` `6e72d26`; quinn-proto 0.11.18; rustc 1.96.0; DarkSide, Linux
  6.12.115.
- Survey: [Stage41 survey](STAGE41_QUIC_INITIATOR_BOUNDS_SURVEY.md), owner
  decisions S41-D0–D5 all as recommended (Option A, connection window
  1.25 MB, discovery shares the config, §8 regression scope, Fang
  connection ceiling later).

## 2. The test

`quic_fang::tests::initiator_grants_the_receiver_no_streams_or_datagrams`
(`crates/werewolfd/src/quic_fang/tests.rs`):

- builds the **production** `client_endpoint` against a test receiver
  pinned to its Pelt key;
- checks that the initiator's own bidirectional stream still carries
  data (the data path must be unchanged by M2);
- then records every exposure before asserting, so a failure names all
  of them:
  - the receiver can open a unidirectional stream toward the initiator;
  - the receiver can open a bidirectional stream toward the initiator;
  - the initiator advertises QUIC DATAGRAM support (a datagram is sent
    to prove it).

Result on `main`:

```
initiator exposure: [
    "receiver can open unidirectional streams toward the initiator",
    "receiver can open bidirectional streams toward the initiator",
    "initiator accepts QUIC datagrams it never reads",
]
```

The connection receive window cannot be observed by this test. With no
peer-opened streams, only the initiator's one stream can receive data,
so the window has no effect on its own. M2 sets it as defence in depth.

## 3. Measurement (throwaway, not committed)

A temporary `#[ignore]` test in the same module made a receiver open 100
unidirectional and 100 bidirectional streams toward a production
`client_endpoint` and write 1.25 MB to each. The initiator read nothing.
After 8 s:

| Measure | Value |
|---|---|
| Receiver bytes sent (`udp_tx`) | 250 498 494 |
| Initiator bytes received (`udp_rx`) | 249 133 614 |
| Initiator STREAM frames received | 171 808 |
| Test process RSS increase | 503 MiB (both ends share the process, so this includes the receiver's send buffers) |

The receiver can only send what the initiator's flow control allows, so
the initiator accepted and holds ~249 MB of data it will never read. It
is held until the connection closes. The idle timeout does not apply
while the peer keeps sending.

The test was removed after the run. Only the M1 test above is committed.

## 4. Checks

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy -p werewolfd --all-targets -D warnings`: PASS.
- `cargo test --workspace`: 413 passed, 0 failed, 1 ignored (the M1
  test).
- `cargo test ... -- --ignored` on the M1 test: FAILS, as designed.

## 5. Next

M2: Option A in `client_endpoint` only. Peer-opened bidi and uni streams
are set to 0, the datagram receive buffer is disabled, and the
connection receive window is 1.25 MB. The stream receive window and the
send window are not changed. M2 removes the `#[ignore]`.
