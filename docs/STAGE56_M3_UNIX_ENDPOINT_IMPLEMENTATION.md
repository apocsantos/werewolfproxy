# Stage56 M3 — Unix local Fang endpoint

Stage56 M3 implements the Linux Unix stream endpoint approved in the merged
M2 design. TCP endpoints and the peer wire protocol remain unchanged.

## Operator contract

Use `unix:/absolute/path/to/fang.sock` as the local endpoint when opening a
Fang or saving its profile. The parent directory must already exist, be a
real directory rather than a symlink, have the setgid bit, and not be writable
by group or others. The daemon does not create or change the parent directory.
It creates the socket with mode `0660` and the directory's group. Stale socket
paths are reported and left for operator cleanup; shutdown removes the path
only if it still names the socket inode created by that listener.

The listener reads Linux `SO_PEERCRED` and charges the connection to that
effective UID's existing share budget of 16. The 64-per-Fang and
1024-per-daemon limits and 60-second own-idle eviction rule are shared with
TCP. A rejected Unix stream closes immediately. The accepted Unix stream flows
directly through the existing Fang transport; no TCP bridge or extra session
descriptors are created.

Unix endpoints are Linux-only. Other platforms continue to accept TCP local
endpoints and reject the `unix:` form. Filesystem paths are absolute, limited
to 512 bytes, and may not contain `.` or `..` path components.

## Implementation and coverage

The shared acceptor now handles TCP and Unix listeners, with identity keys for
canonical IP prefixes or peer UIDs. The listener abstraction is used by QUIC,
encrypted TCP, plain TCP, routed and failover Fangs. The existing copy/setup
functions accept generic async streams; TCP-specific socket options and RST
handling remain on TCP streams.

Coverage includes legacy TCP admission tests, UID key isolation and share
enforcement, live peer-credential attribution, socket type/mode/group checks,
unsafe path and parent rejection, stale-path refusal, replaced-path cleanup,
profile endpoint syntax. At the M3 merge, per-transport Unix end-to-end and
multi-UID namespace coverage were outstanding; the M4 [regression report](STAGE56_M4_UNIX_TRANSPORT_REGRESSION.md)
now records both the five-path exact-byte gate and real UID 0/UID 1 credential
test.

`werewolfctl service connect` accepts `--local-endpoint unix:/absolute/path`
for these endpoints; its existing `--local-port` behavior is unchanged.

## Validation

- `cargo test --locked --offline --workspace`: passed, with 95 core tests, 9
  CLI tests, and 402 daemon tests. The 10,000-session short-lived QUIC test
  took about 245 seconds.
- `scripts/ci-local.sh --fast`: passed (`target/ci-local/20261123T210622Z`).
- `python3 -B tests/integration/unix_fang_gate.py --keep-temp`: passed outside
  the sandbox on the merged M3 code; all five paths returned exact payloads,
  removed their socket paths on close, and left all daemons alive.
- `cargo fmt --check` and `git diff --check`: passed.

The sandboxed gate attempt failed before listener setup because its root
filesystem reports `/` as UID/GID 65534, which the daemon's existing private
filesystem guard rejects. The successful run used the host filesystem view.
