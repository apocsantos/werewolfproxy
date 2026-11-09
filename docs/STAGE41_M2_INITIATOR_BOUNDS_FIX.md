# Stage41 M2 — QUIC initiator resource bounds (fix)

**M2: PASS.** `client_endpoint` now grants the receiver no stream credit
and no datagram buffer, and bounds the connection receive window. The M1
test passes. The receiver-controlled data the initiator holds on one
connection drops from **249 MB to 1.29 MB**. Every existing check
passes.

## 1. Change (`5449261`)

Only `crates/werewolfd/src/quic_fang.rs`, function `client_endpoint`
(Option A, owner decisions S41-D1/D2):

| Parameter | Before (quinn default) | After |
|---|---|---|
| Peer-opened bidi streams | 100 | **0** (`PEER_BIDI_STREAMS`) |
| Peer-opened uni streams | 100 | **0** (`PEER_UNI_STREAMS`) |
| Datagram receive buffer | 1.25 MB | **disabled** |
| Connection receive window | unlimited | **1.25 MB** (`CONNECTION_RECEIVE_WINDOW`) |
| Stream receive window | 1.25 MB | unchanged |
| Send window | 10 MB | unchanged (Stage34 surface) |
| Keep-alive, idle timeout | 10 s, 30 s | unchanged |

The connection window equals the stream window, so the one stream that
carries data is not limited further. The comment above the constants
points to the receiver's bounds in `transport/quic.rs`.

Every `client_endpoint` caller gets the change: the direct QUIC Fang,
failover, the routed QUIC hop and discovery (S41-D3). The M1 test's
`#[ignore]` is removed.

## 2. Results

### 2.1 M1 test

`initiator_grants_the_receiver_no_streams_or_datagrams`: **PASS.** The
receiver cannot open uni or bidi streams toward the initiator, the
initiator advertises no DATAGRAM support, and the initiator's own stream
still carries data.

### 2.2 Measurement (throwaway, not committed)

This is the M1 measurement again, with one addition. The receiver also
writes 4 MB into the initiator's own stream, which the initiator does
not read, so the bounded path is exercised too:

| | M1 (`main`) | M2 |
|---|---|---|
| Initiator bytes received in 8 s | 249 133 614 | **1 285 623** |
| Initiator STREAM frames received | 171 808 | 881 |

On M2, all 200 receiver-opened streams got no credit. The 1.29 MB is
the initiator's own stream, filled up to its window.

### 2.3 Existing checks

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --workspace --all-targets --all-features -D warnings`:
  PASS.
- `cargo test --workspace --all-features`: **414 passed**, 0 failed, 0
  ignored. That is 413 plus the M1 test.
- `scripts/ci-local.sh` full (`target/ci-local/20261109T154229Z`): **13 of
  13 PASS**. This includes acceptance (50 MiB over QUIC and both TCP
  transports), the routed-Fang gate (64 MiB full duplex on both hops) and
  the discovery gates, which use `client_endpoint`.

## 3. Not done here

- M3: the Stage32 regression subset from the survey §8 (S2 P0/P1/P2/P5,
  S3 P0/P1, S4 P0/P2, S5 P0, S7 P0). S3-P2 is not a criterion.
- No change to the data-path windows, to `transport/quic.rs`, or to any
  dependency.
