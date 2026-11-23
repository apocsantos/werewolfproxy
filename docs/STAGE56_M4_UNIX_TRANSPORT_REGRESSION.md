# Stage56 M4 — Unix Fang transport regression

## Scope

M4 closes the M3 per-transport Unix-stream coverage gap and makes the endpoint
usable from the public `service connect` CLI. `--local-endpoint` accepts a
complete endpoint such as `unix:/run/werewolf/web.sock`; the existing
`--local-port` and `--bind` options retain their TCP behavior and cannot be
combined with `--local-endpoint`.

The new disposable [integration gate](../tests/integration/unix_fang_gate.py)
opens a Unix listener for each Fang kind, connects over AF_UNIX, exchanges an
exact echo payload, closes the Fang, and checks that its socket path is
removed. It covers direct QUIC, direct encrypted TCP, direct plain TCP,
routed QUIC-hop, and provider failover.

## Evidence

Run on 2026-11-23 from the M4 working tree based on merged M3 commit
`5eeafe22537a7ac3fb1f7a7ab605d07e7e73d591`; the gate built the current
working sources before starting the daemons:

```text
python3 -B tests/integration/unix_fang_gate.py --keep-temp
PASS direct-quic: Unix stream over quic (17 bytes, exact)
PASS direct-tcp: Unix stream over tcp (26 bytes, exact)
PASS direct-plain: Unix stream over tcp-plain (22 bytes, exact)
PASS routed-quic-hop: Unix stream over quic (19 bytes, exact)
PASS provider-failover: Unix stream over quic (21 bytes, exact)
PASS all five Unix Fang transport paths remained live
```

The gate confirmed each socket path disappeared after `fang close` and all
three daemon processes and the target remained alive through the matrix. The
successful run used the host filesystem view at `/tmp/wwp-lab-xyjsgo5u`.
The existing service registry gate also passed on the same branch, covering
the unchanged `--local-port` CLI path and its QUIC/encrypted TCP behavior.

A private user-namespace test also passed with the Fang acceptor running as
UID 0 and a child client running as UID 1. The acceptor observed
`ClientIdentity::UnixUid(1)` from the kernel credentials, then exchanged a
byte with that client:

```text
unshare --user --map-root-user --map-auto \
  /bin/bash -lc 'cargo test --locked --offline -p werewolfd --bin werewolfd \
  unix_peer_credentials_distinguish_kernel_uids_when_root -- --nocapture'
PASS, 1 test; client SO_PEERCRED UID 1
```

The reusable `python3 -B tests/integration/unix_uid_gate.py` wrapper also
passed in the host filesystem view; its sandboxed invocation reports a skip
when the sandbox blocks the UID map.

The workspace suite passed with 95 core, 10 CLI and 402 daemon tests; the
10,000-session QUIC stress case took 247.66 seconds. After adding the
namespace-specific harness, all 44 Fang-acceptor tests passed, as did all 10
CLI tests and the separate namespace gate. The fast local CI gate, format
check and whitespace diff check also passed.

The sandboxed run failed before daemon readiness: the sandbox maps `/` to
UID/GID 65534, which the existing `PrivateDirectory` ancestor check rejects.
This is why the native integration gate must run with the host filesystem
ownership visible.

## Limits

Windows and non-Linux Unix remain unsupported for this endpoint. The namespace
test proves UID attribution across two mapped UIDs; the independent share
limits are also covered with deterministic identities. The five-path lab is a
same-UID application-data test, not a throughput or descriptor-cap campaign.
