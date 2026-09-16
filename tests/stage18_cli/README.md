# Stage18 CLI provisioning harness

`provisioning.py` creates two disposable user-mode Dens and an echo target
below a private parent directory. It uses only `werewolfctl` for normal Den,
Pelt, Pack, target-policy, Fang, Silver, migration, and status operations.
The sole direct state corruption is an explicitly stopped copy used to prove
that `werewolfctl doctor` diagnoses a malformed Stage15B selector; it is never
used as an administrative workflow and is removed after the test.

The harness retains no payload capture or private identity material. It prints
only pass/fail labels.
