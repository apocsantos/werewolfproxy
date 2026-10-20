# Stage22R-D09: storage fault certification

Source baseline: `main` at `b911e4146142a81f80291b419e3bc734a870f3e7`,
`werewolfd` `a71ad688ad120e617c1b851969e584ed08175e9dbce18ea55f8daaf9793fb0c9`
(the build certified in D06/D07). Runs executed on DarkSide on 2026-10-20
between 15:40 and 16:00 (`+01:00`), local only. No production code,
protocol, release artifact, firewall setting or VPS state changed.
Historical wolves (PIDs 1270 `wolf-a`, 1273 `wolf-b`) and the Stage22R Den
were untouched.

## Purpose

Persistent security state must fail closed
([V1_PERSISTENCE_CONTRACT.md](V1_PERSISTENCE_CONTRACT.md),
ARCHITECTURE §37): a mutation that clearly fails leaves the previous state
live, an indeterminate commit sets `storage_degraded`, startup rejects a
partial or inconsistent Den before any listener, Silver ON fences the
runtime even if persisting it fails, and Silver OFF opens nothing until
durable commit. D09 checks this against a real process with real
filesystem faults.

## Method

Disposable test infrastructure in `/tmp/stage22r-d09/` (outside the
repository):

| File | SHA-256 |
|---|---|
| `nslab.py` (namespace lab) | `2178e26a0d0f6ac9337cb38c5fd0791e6754436fa7f7cf8749b3e3b3364850ab` |
| `d09.py` (driver) | `9568e861ea642bf981b66451b0db36c3a0f16fd06375cf7c161ca8365dcd2bab` |
| `/tmp/stage22r_duplex_client.py` (unchanged D05B client) | `eb71c0aa4fd26419def19fe2e8f42bca7acef921b74b28fe866b68d01fe9e196` |

Two fresh daemons, initiator A and receiver B, each run in its own
unprivileged user and mount namespace with a private root: a tmpfs owned by
the namespace uid with `/usr`, `/etc`, `/dev` and `/proc` bound in, so the
Stage11B ancestor checks hold (the host's real root is unmapped inside the
namespace and would correctly be rejected). Each Den is its own 16 MiB
tmpfs at `/den`. A holder process keeps the namespace alive across daemon
restarts. Daemons run with every capability dropped (`setpriv
--inh-caps=-all --bounding-set=-all --no-new-privs`), so permission checks
apply to them; faults are injected from outside with `nsenter`.

Provisioning followed `packaging/INSTALL.md`: `den init`, `pelt init`,
`state manifest-migrate`, `silver off`; then mutual `pack add`, B's
`target allow` for a local echo target, and an encrypted-TCP Fang profile
on A. Every fault is undone and the daemon verified healthy, with an exact
echo session, before the next.

Faults:

| Fault | Injection |
|---|---|
| Disk full (`ENOSPC`) | Fill the Den tmpfs with a file until no space is left |
| Den not writable | `chmod 500 /den` |
| Read-only Den (`EROFS`) | `mount -o remount,ro` of the Den tmpfs |
| Torn or altered protected state | Edit Den files with the daemon stopped |

## Results: faults while running

During F1 and F3 a 64 MiB echo session was running through A, five
mutations were attempted on A (`pack set-address`, `target allow`,
`fang create`, `service add`, `discover advertise`), then a new session was
opened; afterwards the fault was cleared and one mutation retried.

| Test | Mutations | State during fault | Sessions | After clearing | Verdict |
|---|---|---|---|---|---|
| F1 disk full | 5/5 rejected, exit 70 "local state mutation rejected" | Generation 5 → 5, all five lists unchanged, `storage_degraded=false`, `READY` | Running: exact; new: exact | `target allow` succeeds, generation 6 | PASS |
| F3 Den not writable | 5/5 rejected, same | Generation 7 → 7, lists unchanged, not degraded, `READY` | Running: exact; new: exact | Succeeds, generation 8 | PASS |
| F4 Silver ON on B while B's Den is full | `silver on` reported failure (exit 70) | B `locked=true`, `active=0`, not degraded | Running 256 MiB session cut (40–44 MB delivered); new session refused | — | PASS |
| F5 Silver OFF on B, Den still full | `silver off` rejected (exit 70) | B stays `locked=true` | New session refused | — | PASS |
| F4/F5 recovery | `silver off` after freeing space succeeds | `locked=false`, generation 5 | New session exact | — | PASS |

F4 is the contract's "a failed persistence leaves the runtime fenced": the
command reports the persistence failure, yet the fence is already in place
and stays until a durable `silver off`.

**Read-only Den while running was not run.** The kernel refuses to remount
a filesystem read-only while any file on it is open for writing, and the
daemon holds its Den lock file open for writing for its whole lifetime
(`mount: /den: mount point is busy`). A read-only Den is therefore tested
at startup only (S1). A disk that the kernel forces read-only after an I/O
error (`errors=remount-ro`) cannot be produced without root.

## Results: faults at startup

Each case: stop A, inject, start A, observe for 10 s; then restore the Den
from a copy, restart and verify `READY` and an exact echo session.

| Test | Startup | Listener | Log | Verdict |
|---|---|---|---|---|
| S1 Den read-only | Exit 65 | None | `SECURITY_STATE Read-only file system (os error 30)` | PASS |
| S2 Den full | `READY` | Yes | — ; a mutation is then rejected, not degraded | PASS |
| S3a selected slot document missing | Exit 65 | None | `invalid protected state generation` | PASS |
| S3b selected manifest truncated | Exit 65 | None | same | PASS |
| S3c selector names generation 999 | Exit 65 | None | same | PASS |
| S3d manifest-mode marker missing | Exit 65 | None | same; no fallback to legacy flat files | PASS |
| S3e selector missing | Exit 65 | None | same | PASS |
| S3f selected target policy replaced by other valid JSON (`[]`) | Exit 65 | None | same; the manifest commitment catches it | PASS |
| S4a `contacts.json` malformed | `READY` | Yes | — | As documented (lenient cache) |
| S4b `advertise.json` malformed | Exit 65 | None | `invalid discovery state` | As documented (strict) |
| S4c `services.json` malformed | Exit 65 | None | `invalid persistent security state` | As documented (fail-closed startup validation) |

In an earlier run the S4 files were created with the shell's umask
(mode 0664); all three then refused startup with `unsafe persistent state`,
which shows the Stage11B file-mode check also guards these files. That run
did not test malformed content and is not counted; the table above uses
files created with mode 0600.

## Not covered

- **Indeterminate commit** (`storage_degraded`): the rename succeeded but its
  durability cannot be established. Every fault above failed *before* the
  commit boundary, so each mutation was cleanly `NotCommitted` and the
  daemon correctly never entered degraded mode. Producing a failure between
  rename and directory sync needs a block device that fails I/O on demand
  (for example `dm-flakey`), which needs root; the classification is covered
  by the unit test in `crates/werewolf-core/src/local_fs/atomic.rs`.
- Read-only Den while running (above).
- Faults on the VPS service Den.

## Certification conclusion

Every storage fault that could be injected without root failed closed: no
rejected mutation changed live state, advanced the generation or disturbed
forwarding; Silver ON fenced the runtime even when its persistence failed
and Silver OFF stayed refused until durable; every torn, altered, missing or
read-only protected state refused startup with no listener and no legacy
fallback; and each documented strict or lenient auxiliary file behaved as
documented. 14 of 14 judged checks PASS, 3 recorded as documented.

Stage22R-D09: **CERTIFIED** for clean-failure and startup faults; the
indeterminate-commit path remains certified only by unit test.
