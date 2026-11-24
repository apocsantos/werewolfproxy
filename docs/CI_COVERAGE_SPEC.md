# Hosted CI coverage specification

## Status

Implemented on `main` through PR #185, 2026-11-24. This record describes the
hosted coverage added to the Rust CI workflow.

## Problem

Before PR #185, GitHub Actions ran Rust formatting, Clippy, workspace tests
and a release build. Python integration unit tests and the disposable checkout
acceptance runner were only exercised by `scripts/ci-local.sh`.

## Workflow behavior

Run these two existing commands in the hosted Rust CI test job for every pull
request and push:

```sh
python3 -B -m unittest discover -s tests/integration -p 'test_*.py'
python3 -B tests/integration/run.py
```

Keep the existing Rust checks and their outcomes unchanged. Reuse the test
job's checkout, Rust toolchain and Cargo cache. The acceptance runner builds
its binaries under the ignored `target/stage1-lab/` path using locked offline
dependencies, exercises fresh local fixtures, and checks that the checkout
did not change during the run. A Python failure or acceptance failure must
fail the workflow check.

## Boundaries

- Do not add Python packages or Cargo dependencies.
- Do not run `scripts/ci-local.sh` wholesale in hosted CI. Its audit and
  disposable service gates have different setup and runtime needs.
- Do not run Stage32 network-namespace/netem campaigns, the real UID-map gate,
  privileged host checks, or WAN campaigns on hosted runners. Keep those in
  their existing local or separately provisioned validation paths.
- Do not change production code, transport limits, security policy, advisory
  decisions, release certification, or historical verdicts.
- Do not make the Rust test job depend on network access beyond what Cargo's
  existing dependency resolution requires. The acceptance runner itself uses
  loopback-only fixtures and requires cached locked dependencies.

## Implementation checks

1. The two commands run in the existing pull-request and push workflow on
   `ubuntu-latest`.
2. They run sequentially in the Rust test job, so the acceptance processes do
   not compete with other local-network fixtures.
3. Python unit-test or acceptance failure makes the workflow fail with the
   failing command visible in the job log.
4. Rust formatting, Clippy, workspace tests and release build remain enabled.
5. No hosted job runs a test that requires network namespaces, netem, a second
   UID mapping, or an external host.

## Hosted result

PR #185's Rust CI run #632 passed formatting, Clippy, release build, the
Python integration unit tests, the full workspace suite, and the disposable
checkout acceptance suite. The acceptance runner completed its fixtures,
including the 50 MiB encrypted-TCP transfer and daemon restart/profile
restoration checks, in about 45 seconds. The local pre-push fast gate also
passed.

## Limits

The commands also run under `scripts/ci-local.sh`; the documented lab
prerequisites are Linux, Python 3.11+, Bash, curl, Git, Cargo/Rust, permitted
TCP/UDP loopback and Unix sockets, and a populated Cargo cache. Hosted runs
establish behavior on the Ubuntu runner used for that workflow run only. They
do not extend the project's platform certification or replace namespace,
fault-injection or real-WAN evidence.

The disposable acceptance run builds a second binary set under
`target/stage1-lab/`, so it adds compile and fixture runtime to the existing
test job. Keep the commands sequential; revisit a separate job only if
measured runtime or failure isolation makes that useful.
