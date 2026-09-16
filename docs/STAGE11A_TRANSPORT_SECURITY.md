# Stage 11A transport security contract

Stage 11A changes local transport selection and the locked Quinn security dependency.
It does not change application wire messages, cryptographic primitives, Pack/Pelt
semantics, target authorization, plain TCP forwarding, TLS verification, Fang
lifecycle, or persistence formats.

## Selection and authorization

`wolf-b auto` defaults to `secure`, which is an alias for `strict`.
`WEREWOLF_TRANSPORT_POLICY` may select a policy locally; an empty value is invalid.
Accepted names are exactly `secure`, `strict`, `compatibility`, and `legacy`.
Unknown names, missing/empty policy arguments, and unknown flags return generic
unavailable output and exit 2 before health probes. Success exits 0.

| Policy | Probe and selection order | Plain eligible |
| --- | --- | --- |
| secure / strict | QUIC v3, encrypted TCP v3, fail closed | Never |
| compatibility without flag | QUIC v3, encrypted TCP v3, fail closed | No |
| compatibility with `--allow-plain-fallback` | QUIC v3, encrypted TCP v3, plain | Yes |
| legacy | QUIC v3, encrypted TCP v3, plain | Yes; deliberately weaker |

The compatibility flag is invocation-scoped. It is not saved, read from an
allow-plain environment variable, inferred from failures, or negotiated remotely.
Passing it with strict does not enable plain. `ready` can forward the explicitly
supplied flag to its selector subprocess, without persisting it.

Eligibility is fixed before health probing. Selection does not consult latency,
scores, learned preference, or history. Secure transport failure, regardless of
whether caused by authentication, replay, pinning, target policy, TLS, protocol
compatibility, or availability, never widens eligibility. Under strict and
unauthorized compatibility, the plain URL is not probed at all by `auto`.

`legacy` preserves the historical default ordered fallback, not the former
separate score-based policy variants. `performance`, `stealth`, `resilience`,
`learned`, and `recent` are rejected as selector policy names. Score/history
commands remain diagnostics; their displayed selection now uses strict.

Each invocation starts again at QUIC. There is no retained selection or downgrade
state, so restoration is checked immediately on the next invocation.

Explicit direct plain forwarding remains available. Diagnostic commands such as
benchmark, score, doctor, and ready's diagnostics may inspect all configured
transports independently of selection. The zero-plain-probe guarantee applies to
strict `auto` selection, not every diagnostic command.

The selector relies on correct local configuration of its transport URLs. HTTP
health success alone does not attest which implementation is behind an arbitrarily
configured URL. The local owner must map the encrypted URLs to the corresponding
v3 Fangs. This stage does not create a new transport-attestation mechanism.

## Local identifier naming debt

`tcp-encrypted-v2` and `TCP encrypted v2` remain legacy LOCAL selector identifiers
for caller compatibility. The actual encrypted TCP handshake is v3. The suffix
must not determine security, capabilities, compatibility, or negotiated version.
A later identifier migration should remove this naming debt.

## Selector JSON schema version 2

All `auto --json` results contain these fields, including exhaustion results:

| Field | Meaning |
| --- | --- |
| schema_version | Integer 2 |
| transport | quic, tcp-encrypted-v2, tcp-plain, unavailable |
| label | Existing local display label, or unavailable |
| url | Selected configured URL; null on failure |
| healthy | Whether a transport was selected |
| policy | Requested local policy name |
| policy_class | strict, compatibility, legacy, invalid |
| security_class | authenticated-encrypted, plaintext, unavailable |
| plaintext | True exactly for a selected plain transport |
| fallback | True for selected encrypted TCP or plain; false for QUIC/failure |
| security_downgrade | True exactly for selected plain; false for QUIC/encrypted TCP/failure |
| plaintext_authorization_required | True for compatibility policy, whether or not plain is reached |
| plaintext_authorized | Whether this invocation supplied the explicit flag; not a statement of eligibility |
| fail_closed | True when no selection is returned |

Strict never returns `security_downgrade: true`. The emit path checks plain
eligibility independently before emitting any successful plain result.

Example strict exhaustion:

```json
{"schema_version":2,"transport":"unavailable","label":"unavailable","url":null,"healthy":false,"policy":"secure","policy_class":"strict","security_class":"unavailable","plaintext":false,"fallback":false,"security_downgrade":false,"plaintext_authorization_required":false,"plaintext_authorized":false,"fail_closed":true}
```

Example explicitly authorized compatibility plain selection:

```json
{"schema_version":2,"transport":"tcp-plain","label":"TCP plain fallback 🟠","url":"http://127.0.0.1:9021","healthy":true,"policy":"compatibility","policy_class":"compatibility","security_class":"plaintext","plaintext":true,"fallback":true,"security_downgrade":true,"plaintext_authorization_required":true,"plaintext_authorized":true,"fail_closed":false}
```

JSON serialization uses jq, including escaping local policy names and URLs.
Output is local CLI/control information, not network application wire data.

## Caller migration

| Caller | Migration |
| --- | --- |
| wolf-b auto | Default strict; rejected scoring-policy names require an explicit new policy choice |
| wolf-b ready | Retains structured selection on exit 2 and unhealthy doctor output; validates policy before diagnostics |
| wolf-b policy-explain | Correct policy descriptions; handles exhaustion; no preliminary score probes |
| wolf-b policy-test | Lists the four accepted names |
| wolf-b doctor | Checks secure/strict rather than removed resilience selection |
| wolf-b score / score-history | Diagnostic data retained; recommendation uses strict |
| wolf-dashboard.sh | Displays unavailable selector output instead of treating exit 2 as an unknown response |
| chaos-gate.sh | QUIC loss requires encrypted fallback; no plain success under secure |
| chaos-gate-extended.sh | Both secure transports lost means exhaustion; explicit compatibility/legacy checks retain plain coverage |
| rc-gate.sh | Explicit strict and legacy checks replace old scoring-policy assumptions |
| werewolf-help.sh | Lists new policy choices and explicit compatibility authorization |
| tests/integration/run.py | Separate strict, compatibility without/with opt-in, and legacy matrices; restoration for all |

Existing callers that depended on automatic plain fallback under secure must
choose compatibility with the invocation flag or deliberately select legacy.
Callers must accept exit 2 with valid unavailable JSON. Do not silently retry with
legacy or add the plaintext flag in response to failure.

The operational chaos/RC scripts are not executed against installed services by
the isolated certification lab. Their transport behavior is covered by the
isolated acceptance matrix; script syntax is checked separately.

## Dependency remediation and provenance

Only quinn-proto changes from 0.11.14 to 0.11.15. Quinn remains 0.11.9,
quinn-udp remains 0.5.14, and rustls remains 0.23.40. No Cargo.toml changes.

RUSTSEC-2026-0185 / CVE-2026-25800 reports unbounded out-of-order stream reassembly
memory exhaustion. The RustSec patched range is >=0.11.15:
https://raw.githubusercontent.com/RustSec/advisory-db/main/crates/quinn-proto/RUSTSEC-2026-0185.md

The first precise Cargo resolution rewired nine Windows edges and was rejected.
The accepted candidate preserves the committed resolution, changing only the
quinn-proto version and authentic registry checksum. It was validated in /tmp
with locked metadata, Linux/Windows dependency and feature comparisons, and Linux
workspace/all-target checks before application to the real repository.
Windows compilation was unavailable because the Windows Rust target was absent;
no SDK or target was installed. Both platform graphs were validated.

Accepted Cargo.lock SHA-256:
`c44b36ac58fed360590818da27ae2d2441a37467bf7434c3861d8b348ddae2c5`.

Ordinary locked builds preserve this resolution. A future partial cargo update
may produce a different compatible resolution; byte-for-byte update reproduction
is not claimed. Review future dependency-edge changes independently.

A direct vulnerability pseudo-exploit was intentionally omitted. Evidence uses
exact lock provenance, upstream patched-version status, complete QUIC regression
coverage, and the extended exporter-property test: endpoint equality, connection
separation, server recreation separation, label/context separation, key-update
stability, and ACK substitution rejection. Existing daemon restart/crash, replay,
pinning, authorization, admission and malformed OPEN tests remain mandatory.

## Validation and rollback

Mandatory gates are fmt, workspace/all-target Clippy, locked/offline Rust tests,
Python integration unittest discovery, full isolated acceptance, and diff checking.
Final certification runs from the clean committed tree. Source hashes are checked
across every gate. Socket tests require local socket permission; sandbox EPERM is
an environment restriction, not a passing test.

Selector tests use a fake curl and record every probe. They verify strict/plain
exclusion, compatibility invocation scope, default policy, invalid names/arguments,
high plaintext history scores, ignored environment opt-in, restoration, schema
flags, JSON escaping, and wrapper exhaustion behavior. The live lab preserves
application traffic, 50 MiB hash validation, binary forwarding, profile restoration,
and all security-policy sequences.

Rollback must use reviewed new commits, never rewrite certified history. Keep the
patched lockfile independently of selector rollback. Do not silently restore
permissive behavior under the name secure. A dependency rollback to vulnerable
0.11.14 is not an acceptable security rollback; investigate a patched alternative.

## Explicitly deferred requirements

Stage 10 clear TCP handshake metadata still fails the wire-opacity requirement.
Binary encoding alone would not provide identity/target confidentiality. Hide and
protocol metadata confidentiality require a dedicated design.

Control-plane hardening, Den/persistence hardening, receiver-session revoke/Silver
semantics, traffic morphing, TLS certificate verification changes, and broader
protocol/lifecycle work are deferred. Pack membership remains identity
 authentication only, never target or plaintext-fallback authorization.
