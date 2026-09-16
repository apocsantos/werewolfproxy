# Stage 13E volume-only model

`volume_study.py` reads the committed Stage13D production TCP
`sessions.csv`: 275 independent connection summaries with measured TLS wire
bytes in each direction. It retains only public metadata in
`control_dataset.csv`, aggregate candidate metrics, classifier results and
cost summaries. There is no raw capture, application payload, key or secret.

From the repository root:

```sh
python3 -B tests/stage13_volume_study/volume_study.py
```

The candidate edges, modes, randomized probabilities, seed, classifier
features and decision thresholds are frozen in `policies.json`. The script
first reproduces the unmodified control and checks the Stage13D passive
scores, then evaluates alternatives using the unchanged Stage13A/B/C
four-class, per-workload 70/30 split and train-only-scaled nearest-centroid
classifier. The seed is 13.

Every result is an **optimistic final connection-total byte target**. It
does not specify when or how non-application bytes could be sent. All modeled
padding leaves the original pre-tail transfer unchanged. Current Stage10
has no authenticated arbitrary cover-frame semantics, and none are added by
this study. `F_constant` uses the maximum observed connection or directional
total and is a theoretical bound, not an online policy.
