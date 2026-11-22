# Stage55 M3 — full-cap regression

**Status: validation in progress.** M2 PR #175 merged as
`16d67a572a471f090f2e95ab0cca78588e92b42b`. M3 adds native controls at the
real 1024 global-session ceiling and reruns the reviewed Stage54 regression
and local-gate scope. No runtime policy or cap change is part of this step.

## New full-cap controls

The native fixture uses generated local nodes, loopback targets, a 4096
task-owned descriptor limit and real 65-second idle observations. It does not
lower the global limit or receiver quotas. Each direct case holds 32 sessions
on the owner and 992 on independent plain-TCP siblings. The routed-owner
variant sends the secure owner through a second generated node. The concurrent
variant starts twelve simultaneous owner probes after idle and retains every
success so each reclaimed slot remains occupied during verification.

Every case checks actual echoes, owner-victim counts, sibling and empty
Fang protection, recent-session protection, the local-64 positive control,
release-one recovery, receiver authority, daemon survival, idle CPU, descriptor
drain and owned-process cleanup. Concurrency may deny contenders after the
single-victim gate, capacity race or secure setup failure after an idle
victim has been released. Such a failed setup can leave a free global slot;
the fixture records the extra victim and refills the slot before continuing
full-cap controls. It never permits sibling theft.

## Retained regression scope

The campaign retains Stage54 64/32/21 idle shares, direct and routed provider
commitment, source/local/peer interactions, membership changes and storms,
path activation, resolver faults, hostname migration, address/refusal/resource
controls, fairness, S2/S3/S4/S7 P0 chaos, debug/release authentication proofs,
workspace checks and all local-CI gates. Native tests use only disposable
owned processes and private fixture identities.

Evidence and the exact worker results are recorded in the ignored
`target/stage55-m3/` campaign directory. Final counts, hashes and any limits
will be added here only after every worker and the overall campaign manager
has exited successfully.

## Boundaries

These are functional loopback regressions, not throughput or comparative
liveness measurements. They do not establish cross-Fang fairness, WAN behavior
or universal admission under races. Frozen release artifacts, dependency
versions, advisory exceptions and operator settings remain unchanged.
