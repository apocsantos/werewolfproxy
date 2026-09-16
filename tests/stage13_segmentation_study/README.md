# Stage 13D ordered TCP frame metadata

`capture_frames.py` reuses the Stage13C production encrypted-TCP relay and
length-only daemon diagnostics. It retains `frame_dataset.csv.gz`: ordered
payload lengths, observed padded lengths, direction and per-direction frame
sequence. It also retains 275 per-connection public wire summaries in
`sessions.csv`. No payload content, raw TLS capture, private key or traffic
secret is retained. Disposable Dens and raw daemon logs are deleted by the
capture fixture.

The production source HEAD for this dataset is recorded in
`capture_manifest.json`. On a host with a Stage11B-safe temporary ancestor:

```sh
python3 -B tests/stage13_segmentation_study/capture_frames.py
python3 -B tests/stage13_segmentation_study/segmentation_study.py
```

The certification sandbox's `/tmp` was owned by an untrusted UID, so its
daemon correctly refused to use it. Capture used a temporary private 0700
directory under the user's home through `TMPDIR`; no Den safety rule was
disabled. The temporary directory was removed after the run.

`policies.json` fixes all chunk capacities, probabilities, seed, classifier
ablations and decision thresholds. The simulator first asserts exact
per-connection application bytes, directional frame counts and wire-byte
reconstruction for control A. Online candidates only split a read result;
they never wait for future bytes. `OFFLINE_IDEAL_MAX` uses complete future
stream knowledge and cannot be deployed as an online no-delay algorithm.
The inherited Stage13A four-class, 70/30 per-workload, seed-13,
train-only-scaled nearest-centroid classifier is used unchanged.

The model's new frames use the existing Stage10 random-padding formula with
seeded simulation draws; unchanged reads retain their measured padding. Wire
byte projections assume the one-frame/one-TLS-record mapping validated in
this capture. That mapping is empirical, not a protocol guarantee.
