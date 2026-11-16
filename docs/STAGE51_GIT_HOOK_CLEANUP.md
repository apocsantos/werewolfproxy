# Stage51 — Git-hook cleanup

**Status: implemented for review, 2026-11-16.** Routine pushes run only
the existing `scripts/ci-local.sh --fast`: Python unit tests and the
advisory audit. The installer no longer places checkout builds, target
servers, `wolf-b heal` or legacy quick/JSON/schema gates in the hook.

## Scope and installation

Baseline is Stage50 closure merge `fcb42ee`. This follows the bounded
task identified in the [Stage50 closure](STAGE50_CLOSURE.md) and the
owner's instruction to proceed. Application source, lockfiles, advisory
ignores, CI gate definitions and hosted workflows are unchanged.

Run `scripts/install-git-hooks.sh` to install or upgrade. It resolves the
repository from its own location and uses Git's hook path, respecting
linked worktrees and relative/absolute `core.hooksPath` settings. A
changed previous hook is saved as `pre-push.backup.*` beside it before
replacement. The new executable is prepared in that directory and moved
into place; an identical reinstall creates no extra backup.

The hook resolves the pushing worktree at runtime and executes its
`ci-local.sh --fast`, preserving the check's exit status. A failed check
blocks the push. Fast CI still requires `cargo-audit`, may refresh its
database over the network, and writes its normal local evidence. Full
local CI and Rust checks remain separate required pre-merge/stage checks.

Pulling does not update installed hooks. Existing clones must rerun the
installer. Replacing an existing custom pre-push hook requires reviewing
its saved copy and manually integrating any checks still needed; the
installer does not chain or run that copy. A configured shared hook path
can affect other repositories using it. No global Git configuration is
changed, and the legacy operator scripts remain available for intentional
manual use, as described in [daily commands](DAILY_COMMANDS.md).

## Validation

`tests/integration/test_git_hooks.py` adds four stdlib unittest cases
using temporary repositories, isolated Git configuration and local bare
remotes. They exercise:

- A real push invoking only `--fast`, followed by a failing gate that
  prevents the remote ref from advancing; direct invocation returns the
  gate's exact exit status.
- Preservation of an existing hook, idempotent reinstall, executable
  permissions and removal of temporary installer files.
- Relative and absolute configured hook paths, including spaces.
- Installation from a linked worktree and real pushes from both it and
  the primary worktree, each selecting its own checkout's gate.

Available legacy commands are replaced with failing sentinels in every
fixture. Any invocation fails the safety assertion. Git's inherited
repository variables and user configuration are isolated so running the
tests from inside a real push hook cannot redirect a fixture to the
developer's repository. All fixture repositories are removed afterward.

Targeted hook tests: **4/4 PASS**. The full Python suite is **68/68 PASS**.
Shell syntax, changed-document links and whitespace checks pass.

| Run | Result | Evidence |
|---|---|---|
| Fast local CI | PASS, exit 0 | `target/ci-local/20261116T173349Z/summary.tsv` |
| Full disposable local CI | **13/13 PASS**, exit 0 | `target/ci-local/20261116T173409Z/summary.tsv` |

The first sandboxed fast attempt passed the unit suite but could not
acquire Cargo's advisory-database lock on its read-only path. It is
retained at `target/ci-local/20261116T172739Z`; the authorised rerun above
passed. No advisory exception or gate was changed. The full suite ran
without tree edits; only this results documentation was completed after
it finished.

## Boundaries

No installed node, private Den, real peer, service unit, WAN host or
persistent target server is used. Frozen rc.1/rc.2 artifacts, S3-P2/S2-P4
exclusions, Stage38 closure and Tokio 1.52.3 remain unchanged. This task
does not resolve GitHub's billing block or certify transport behaviour.
