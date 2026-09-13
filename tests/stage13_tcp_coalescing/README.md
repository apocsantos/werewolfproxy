# Stage 13B TCP outer-TLS write experiment

The production change is confined to `write_encrypted_frame`: each existing
Stage10 ciphertext-length prefix and ciphertext are submitted with one
`write_all()`. The Stage13A transparent relay measures public TLS record
boundaries. It does not terminate TLS or save payloads or secrets.

`summary.json` and `features.csv` contain 275 post-change TCP connections:
30 for each non-bulk workload and five 50 MiB bulk downloads. The source
capture and per-unit CSVs were analyzed in `/tmp`, then discarded. The
comparison in `comparison.json` uses a second 275-connection reference run
from an archive of exact Stage13A commit `eb0ac3647e650c6b556ca1a0f3d45e883c247e31`.
Only the TCP-selection option and aggregate analysis fields were copied into
that archive's test tooling; its production source was unchanged. Both runs
used the same workload, capture, classifier and burst definitions. The
original certified Stage13A aggregate remains in
`../stage13_traffic_morphology/summary.json`.

The capture occurred before the Stage13B commits, so its `source_head` field
records the Stage13A parent. The post-change production
`tcp_encrypted.rs` SHA-256 at capture was
`c9efad7d2b4e44fd9e0d9e8739d95316e1d77c0c5ae20b9e547ff43cc98fb7c1`;
the archived reference file SHA-256 was
`7789478db67dd8fe3fec966b5ca0183904e77c1d2ab23625bb5d1dfa9aa89280`.

To repeat the post-change measurement from the repository root:

```sh
python3 -B tests/stage13_traffic_morphology/capture.py \
  --transport tcp --samples 30 --bulk-samples 5 \
  --output /tmp/stage13b-tcp-captures.json
python3 -B tests/stage13_traffic_morphology/analyze.py \
  /tmp/stage13b-tcp-captures.json \
  --output-dir tests/stage13_tcp_coalescing
```

`compare.py` accepts the two analysis `summary.json` files and corresponding
production TCP source files. It records the source hashes and checks matching
lockfile hash, workload sizes, sample counts, capture units and burst rule
before writing a compact comparison. Raw capture bytes, TLS key logs and
private material are never retained.
