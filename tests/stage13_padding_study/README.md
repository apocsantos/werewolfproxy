# Stage 13C frame-size model

`capture_frames.py` runs 275 ordinary production encrypted-TCP connections
through the Stage13A transparent relay. It pairs the daemon's existing local
**length-only** forwarding diagnostics with passive TLS record lengths, checks
one-to-one frame mapping and writes grouped numeric metadata. Its temporary
production Dens and full logs are removed by the fixture. The committed
`baseline_frames.csv` is a lossless grouped multiset of 351,055 frames, not a
raw wire capture or plaintext-content archive. `baseline_sessions.csv` holds
the 275 corresponding public wire-feature rows.

From the repository root, on a host with a Stage11B-safe temporary ancestor:

```sh
python3 -B tests/stage13_padding_study/capture_frames.py \
  --samples 30 --bulk-samples 5 \
  --output-dir tests/stage13_padding_study
python3 -B tests/stage13_padding_study/padding_study.py
```

The sandbox used for this certification exposed `/tmp` as owned by an
untrusted UID. The daemon correctly rejected that ancestor. The successful
run used a private 0700 temporary directory under the user's home as
`TMPDIR`; no Den safety check was disabled. A fresh ordinary host with a
root-owned sticky `/tmp` can use that standard location.

`policies.json` was fixed before running candidate classification. The model
replays measured control sizes and first validates every frame's production
padding support, lengths and wire reconstruction. `padding_study.py` then
reuses the Stage13A nearest-centroid classifier and writes
`candidate_metrics.csv`, `classifier_results.json`, `overhead_budget.json`
and `summary.json`. The candidate RNG is deterministic seed 13 for simulation;
production still uses `OsRng`. No application content, private keys or TLS
secrets are retained.
