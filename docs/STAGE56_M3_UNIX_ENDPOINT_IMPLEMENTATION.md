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
and profile endpoint syntax. The full existing transport suite exercises the
unchanged TCP paths. This implementation does not claim multi-UID namespace
testing or per-transport Unix end-to-end coverage.

## Validation

- `cargo test --locked --offline --workspace`: passed, with 95 core tests, 9
  CLI tests, and 402 daemon tests. The 10,000-session short-lived QUIC test
  took about 245 seconds.
- `scripts/ci-local.sh --fast`: passed (`target/ci-local/20261123T210622Z`).
- `cargo fmt --check` and `git diff --check`: passed.
