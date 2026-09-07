# Stage 9 receiver target authorization

Encrypted TCP v2 and QUIC receivers authorize targets using the authenticated
stable Pack fingerprint. Plain TCP connects directly from the initiating daemon
and does not use this policy.

## Configuration

The policy is `<resolved --home>/target_policy.json`. The default is
`$HOME/.config/werewolf/target_policy.json`; the daemon expands its default `~`.
It is read once during startup. File edits require a daemon restart; there is no
runtime reload or policy-control API.

Strict accepted shapes:

```json
{
  "mode": "deny-by-default",
  "peers": {
    "wwp1:01-23-45-67-89-AB-CD-EF": {
      "targets": [{"address": "127.0.0.1", "port": 8080}]
    }
  }
}
```

```json
{"mode": "legacy-allow"}
```

The fingerprint example illustrates syntax, not an identity to deploy. Use the
actual authenticated peer's Pack fingerprint. Fingerprints must match
`^wwp1:[0-9A-F]{2}(-[0-9A-F]{2}){7}$`, the existing Pelt format.

`deny-by-default` requires a `peers` object. Each peer requires a `targets` array;
empty maps/arrays grant nothing. Each target requires exactly `address` (a Rust
`IpAddr` string) and `port` (a JSON integer from 1 through 65535). `legacy-allow`
accepts only `mode`, with no `peers` field, including null. Unknown fields,
duplicate object keys, duplicate canonical endpoints within one peer, malformed
fingerprints/addresses/ports, unsupported rule types, and unknown modes invalidate
the entire policy. No entries are skipped. Grants for different peers may name
the same endpoint without conflict. Hostname, CIDR, wildcard and alias grants are
not supported.

Missing file sets `Deny`. Other read errors, malformed JSON or invalid schema set
`Invalid`. Both deny all peer-mediated requests. The daemon continues startup;
these conditions do not terminate it or currently emit a policy-load diagnostic.
Invalid policy never falls back to legacy mode. A missing file does not imply
legacy mode.

## Resolution, authorization and connection

Literal addresses are parsed as `SocketAddr`; IPv6 targets require brackets.
Policy addresses are parsed as `IpAddr`. Expanded and compressed IPv6 forms are
equal by their numeric address. IPv4-mapped IPv6 remains an IPv6 address, distinct
from IPv4; no address-family conversion grants additional access.

For a hostname, the module parses host and numeric port, invokes Tokio
`lookup_host((host, port))` once, sorts/deduplicates the resulting `SocketAddr`
set, rejects empty/unsupported results, and requires every endpoint to occur in
the authenticated fingerprint's grant set. Any unauthorized result rejects the
whole request. Literal targets bypass DNS. Missing/invalid policies deny before
resolution. No policy decision uses hostname identity, alias, profile, source IP,
peer name, or configured peer address.

Receivers pass only `authorized_targets.as_slice()` to `TcpStream::connect`.
They never pass the original hostname to the connection operation. Tokio tries
the canonical, sorted endpoints sequentially until one succeeds or all fail.
A single deadline, five seconds after starting target authorization, is shared
by resolution/authorization and the entire multi-address connection operation.
It is not renewed per endpoint. Scheduling and synchronous work are subject to
Tokio's cooperative timeout behavior. Cancellation stops the request's wait;
an already-running blocking OS resolver task may finish in the background.

All scope identifiers (`%...`) and IPv6 unicast link-local addresses
(`fe80::/10`) are rejected, including in legacy mode. Policy grants cannot encode
an interface scope safely, so scope identifiers are never stripped. Global IPv6,
ULA and loopback IPv6 need exact grants in deny mode, like all IPv4 addresses.
Legacy mode restores unrestricted authenticated-peer access to supported
endpoints; it does not override target-syntax or scoped/link-local restrictions.

Encrypted TCP retains its existing request parsing, fingerprint derivation,
signature verification, Pack-membership validation and replay validation, all
before target authorization. In the pre-existing implementation signature
verification precedes the Pack-membership check; Stage 9 does not reorder that
frozen authentication logic. It resolves/authorizes the authenticated fingerprint,
connects only to the approved endpoints, and sends the existing ACK and encrypted
traffic only after connection succeeds.

QUIC retains versioned request parsing, Pack-key authentication, signed-request
verification and replay validation. It then resolves/authorizes, connects to an
approved endpoint, sends its signed successful v2 ACK, and proxies application
data. Denial exits before connection and before ACK construction.

Denied encrypted TCP requests close without an ACK; denied QUIC streams close or
reset without a successful ACK. Target-policy errors contain only the generic
`target authorization denied` text locally. TCP target deadlines have generic
local timeout text; QUIC authorization/resolution deadlines use its generic
authorization-denied log. No policy rules, permitted targets or resolver details
are returned to the peer. Existing non-authorization errors are unchanged.

## Validation coverage

`target_policy.rs` has executable unit tests for parsing, canonical literals,
per-peer grants, ungranted loopback/private/public/ULA/link-local addresses,
wrong IP/port, legacy mode and fail-closed configuration. Deterministic injected
resolver tests cover all-authorized, mixed, entirely unauthorized, empty and
failed DNS results, deduplication and one resolver invocation. These DNS-set
cases are fixtures, not host DNS manipulation. Source checks additionally verify
that both receivers use resolved slices and the shared deadline.

`target_authorization_tests.rs` sends real signed TCP and QUIC requests using
fresh identities and loopback listeners. For each transport and each available
IPv4/IPv6 target it exercises explicit grant, missing file, empty grants,
malformed JSON, unknown mode, wrong port, wrong IP, another authenticated peer,
and explicit legacy mode. Both peers share an alias and unrelated configured
address, demonstrating that authorization follows the fingerprint. Each request
must reach replay validation. Success requires a verified receiver-signed ACK
and target acceptance. Denial requires no ACK bytes and no queued/observed target
acceptance after request completion. IPv6 can only be skipped for explicit
address-unavailable/unsupported bind errors, never permission failures.

The isolated acceptance lab uses `deny-by-default` with exact HTTP/echo grants
for the initiating fingerprint. Legacy compatibility is tested separately by the
real receiver tests. Public/private negative unit tests do not connect to those
networks. Zero accepted connections is the live observation; source order also
establishes that denial returns before the sole target connection call.

## Scope

The only dependency change is declaring the already-locked `serde` package as a
direct werewolfd dependency for strict deserialization. No package version or
cryptographic dependency changes. Existing module/function visibility is
unchanged; new parsing helpers and the test module are private.

No changes to authentication algorithms, Ed25519, X25519, BLAKE3 KDF,
ChaCha20-Poly1305, framing/transcripts, receiver pinning, replay semantics, Fang
lifecycle, topology, fallback/scoring scripts, Pack/Pelt semantics, Stage 0–8
persistence formats or TLS SkipServerVerification are part of this completion.
The existing broader authentication/TLS limitations are not certified away by
these authorization tests. Stage 10 is outside this work.

## Completion gates — 2026-09-07

All mandatory gates were rerun on the completed source after reboot:

| Command | Result |
|---|---|
| `cargo fmt --check` | PASS |
| `cargo clippy --workspace --all-targets` | PASS; 12 existing warnings, plus duplicate test-target reports |
| `cargo test --locked --offline --workspace` | PASS; 29 tests, including 13 daemon tests |
| `python3 -B -m unittest discover -s tests/integration -p 'test_*.py'` | PASS; 50 tests |
| `python3 -B tests/integration/run.py` | PASS; all eight acceptance checks |
| `git diff --check` | PASS |

Socket-dependent Rust/Python checks initially encountered sandbox `EPERM` and
were rerun successfully with explicit local socket permission. Failed sandbox
invocations are not counted as passes.

The two real receiver tests exercised 36 cases: nine policy cases on each of TCP
and QUIC, for both IPv4 and IPv6 (no IPv6 skips). All 28 denials observed zero
target connections and no ACK bytes; all eight grant/legacy successes observed
a connection and verified the receiver ACK signature. Legacy passed for a
separately authenticated peer without a grant on both transports/address families.

The isolated acceptance matrix passed QUIC, encrypted TCP and plain TCP exact
51-byte application payloads; secure QUIC → encrypted TCP → plain fallback;
restoration to QUIC; encrypted TCP 52,428,800-byte transfer with SHA256
`80d160f35ab0b90c95b2f4777c0fa0127bb2a4b5f6fff2f4a6c0f3a4c1219ea6`;
16 KiB binary echo over all three transports; and daemon restart with all four
profiles restored and traffic rechecked. Temporary private runtime state was
removed and the harness's source-preservation check passed.

The lab exercised checkout binary SHA256
`f2bb89e7583854370c2e1eddc0b255eb1e79ee710a0cd4ebb18910530dfd2959` and lockfile SHA256
`bca43b42df6f4e3786501652799b58e877ea30d862040ef96741a38405aea562`.
Gates ran before creating the completion commits, against the same final source
and tests; only this documentation result record was added afterward.
