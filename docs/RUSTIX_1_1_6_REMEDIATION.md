# rustix 1.1.6 remediation (RUSTSEC-2026-0486)

Owner decision, 2026-11-11: move the `rustix` pin to 1.1.6 in its own
change, ahead of Stage46 M1. Dependency change only; no Werewolf code
changes.

## The advisory

The local CI audit (`scripts/ci-local.sh`, the pre-push gate) refused the
Stage46 M1 push on 2026-11-11 (`target/ci-local/20261111T114252Z`):

| Advisory | Kind | Affected | Patched |
|---|---|---|---|
| RUSTSEC-2026-0486 (GHSA-jh2m-fhpj-3vr7), 2026-10-27: `getxattr`, `lgetxattr`, `fgetxattr`, `listxattr`, `llistxattr`, `flistxattr` mark unwritten bytes as initialized when given a zero-length buffer | unsound | `>= 1.0.0, < 1.1.6` | `>= 1.1.6` |

**Exposure (code reading):** `rustix` is a direct dependency of
`werewolf-core` only (features `fs` and `process`), used by `local_fs`
for `openat`-style file operations, `stat`, `flock`, rename and the
effective user and group. No Werewolf code calls any xattr function, so
the affected functions are not reached. The remediation does not depend
on this reading.

## The change

| Where | Before | After |
|---|---|---|
| `rustix` in `crates/werewolf-core/Cargo.toml` | `=1.1.4` | `=1.1.6` |
| workspace `Cargo.lock` | 1.1.4 | 1.1.6 |
| `tests/stage12_quinn_auth/Cargo.lock` (through `werewolf-core`) | 1.1.4 | 1.1.6 |

`cargo update -p rustix --precise 1.1.6` changed only `rustix` in each
lockfile (the workspace keeps 218 packages); no other dependency moved.
Workspace `Cargo.lock` SHA-256: before
`a191a20e0e943f726884f5965b93814df4c59145d170de66e3b554ab0b36d27b`,
after `67051e26a5cb30b4d9a045cd673885cb22f9ab8d7d5c881bbe54c158a279f5db`.

**Not changed:** `packaging/SBOM.json` and `packaging/THIRD_PARTY_NOTICES`
describe the frozen `v1.0.0-rc.2` archive, which still ships rustix 1.1.4
and is not rebuilt (rc.3 is not authorised). Any future release inputs are
regenerated from the lockfile.

## Validation (2026-11-11)

- `cargo fmt --check` and `cargo clippy --workspace --all-targets
  --all-features -D warnings`: PASS.
- `cargo audit`: no advisories beyond the accepted RUSTSEC-2026-0386
  (tokio, `.cargo/audit.toml`).
- `tests/stage12_quinn_auth` builds.
- `scripts/ci-local.sh` full: **13 of 13 PASS**
  (`target/ci-local/20261111T114528Z`), including the unit tests, the
  acceptance lab, which exercises `local_fs` on every Den, and every
  gate.
