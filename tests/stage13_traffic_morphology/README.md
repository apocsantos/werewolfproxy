# Stage 13A production traffic morphology measurement

`capture.py` starts two ordinary werewolfd processes with fresh Pelt
identities in a disposable private Den, grants the sender only the local test
target, and opens one production TCP and one production QUIC Fang. The selected
Pack address points through a transparent relay to the actual receiver. Each
sample creates a **new** local application connection and therefore a new
receiver-authenticated secure connection. The relay forwards bytes unchanged,
does not terminate TLS, and records only metadata. The disposable Den,
including production Pelt private material, is removed in `finally`.
The fixture restarts the two daemons between QUIC workload batches while
reloading the **same persisted Pelts and Pack records**. This keeps accepted
samples below the existing 64-per-peer live-connection authority cap; it
does not alter Quinn or Stage 11C resource policy. A metadata checkpoint
after every successful sample permits a failed disposable batch to resume
with a fresh Den and new Pelt pair without counting a failed connection.

Run from the repository root:

```sh
python3 -B tests/stage13_traffic_morphology/capture.py \
  --samples 30 --bulk-samples 5 --output /tmp/werewolf-stage13a-captures.json
python3 -B tests/stage13_traffic_morphology/analyze.py \
  /tmp/werewolf-stage13a-captures.json
python3 -B tests/stage13_traffic_morphology/probe_initial.py
```

The full intermediate JSON and its `*.units/` directory are metadata only
but are kept in `/tmp`; they are not committed. Each connection's unit CSV
records a relative monotonic timestamp, direction and length for **every**
observed relay read or UDP datagram. A second CSV records parsed public TLS
record timestamps, lengths and content types. The committed `features.csv`
and `summary.json` are compact derived artifacts. No raw TCP/UDP payloads,
private keys, TLS traffic secrets,
QUIC secrets, exporter bytes, or key logs are stored by the capture tool.
Payload-integrity comparisons occur in memory.

| Workload | Local application pattern | Samples per transport |
| --- | --- | ---: |
| handshake_only | Connect; wait until authenticated target accept; close without application payload | 30 |
| tiny | Echo 32 bytes | 30 |
| interactive | Sequential echoes of 16, 32, 64, 128, 256, 16, 64, 128 bytes | 30 |
| request_1kib | Echo 1 KiB | 30 |
| transfer_64kib | Echo 64 KiB | 30 |
| transfer_1mib | Echo 1 MiB | 30 |
| bulk_50mib | 32-byte request, 50 MiB download with SHA-256 integrity | 5 |
| asymmetric_upload | 1 MiB upload, 32-byte receipt | 30 |
| asymmetric_download | 32-byte request, 4 MiB download | 30 |
| idle | Connect; wait for authenticated target accept; remain idle one second; close | 30 |

Timestamps use `time.monotonic_ns()`. A burst begins whenever adjacent
observed units are separated by **more than 20 ms**; this threshold was fixed
before measurement. The CSV contains per-connection totals, counts, first 16
lengths with directions, duration, response latency, median and p90
interarrival gaps, burst count and first 32 burst byte totals, largest and
smallest nonempty unit, mean and median size, cumulative size histogram,
direction changes, and up/down byte ratio. The JSON summarizes those features,
public TLS ClientHello/ServerHello fields and public QUIC long-header fields.

For TCP, a unit is one transparent relay `recv()` call. It is **not** an
original sender TCP write, network packet, or IP segment. TLS record
boundaries and lengths are independently parsed from the relayed byte stream.
For QUIC, one unit is one UDP datagram. The passive observer does not use
SSLKEYLOGFILE, TLS/QUIC secrets, or application decryption. Public QUIC Initial
decryption observations from the locked Stage 12 proof supplement the
production datagram-header measurements. This tool does not decrypt
production QUIC Initials, so it does not report transport-parameter values
that a separate standard Initial decoder might obtain. The optional
`probe_initial.py` performs that separate inspection on **one** production
ClientHello using the public QUIC v1 salt and destination connection ID. It
requires the already-installed Python `cryptography` package, authenticates
the Initial AEAD, outputs only public field summaries, and discards the raw
datagram without writing it to disk. It is supplementary to the 550-session
metadata sample; it does not pretend to decrypt later QUIC Handshake or 1-RTT
packets.

`analyze.py` uses Python's standard library. For each transport separately,
it stratifies independent connections **by workload** into a 70/30 train/test
split with deterministic seed 13. It trains a standardized, log-feature
nearest-centroid classifier on four prespecified classes: idle/empty,
tiny/interactive, medium, and bulk/asymmetric. The TCP classifier excludes
relay read counts and sizes, using only direction-specific byte totals, public
TLS record count and duration; the QUIC classifier can use actual datagram
counts and sizes. Standardization uses training data only. A size-only
ablation removes duration and burst/direction timing features. Wilson
intervals describe sampling uncertainty within this fixture, not external
generalization. Reported accuracy is held-out accuracy on the same loopback
host and a small number of disposable daemon identity pairs; it does not
establish Internet-wide recognition or generalization across platforms.

TCP versus QUIC is already visible as TCP versus UDP at IP level. This
measurement compares workload morphology **within** each transport and does
not propose camouflage or transport impersonation.
