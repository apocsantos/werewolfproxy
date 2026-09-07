# Stage 1 limitations and defect record

No production defects are fixed by this lab.

## Existing interface limitations used by the lab

- `pack.add` cannot supply public keys and rejects duplicate fingerprints needed
  for A's QUIC/TCP aliases. Only fresh temporary Pack files are written, in the
  existing format, while both lab daemons are stopped. See
  `crates/werewolfd/src/main.rs` (`pack.add`) and core `pack.rs`.
- Daemon/Fang listeners take explicit ports and do not accept reserved socket file
  descriptors. Releasing a reservation before production binds leaves a small
  port-claim race. Startup failure is reported, never redirected to live ports.
- Fang open success precedes asynchronous listener binding. The harness checks
  listener readiness and then application bytes rather than trusting control
  success alone (`open_fang_from_parts`).
- Closing a Fang aborts its listener; established child streams need not end.
  Fallback probes use new connections, as Stage 0 did. Cleanup terminates whole
  owned process groups.
- CLI human output/exit status is not a reliable structured daemon-error signal.
  This lab uses control response IDs, `ok`, `result` and errors directly.

## Scope limits

This is localhost debug-build acceptance, not cross-host, systemd/watchdog,
release, adversarial authentication or arbitrary transport-failure validation.
It reproduces Stage 0 listener-close fallback, not UDP packet-loss injection.
The production selector has fixed defaults outside its `auto` branch; the runner
uses only `auto --policy secure --json` with explicit private URL overrides.

## Defects discovered during Stage 1 execution

No new production defects were observed in the complete acceptance run on
2026-09-07. Initial socket tests and the first lab invocation were denied by the
execution sandbox (`EPERM`). Rerunning with explicit local socket permission
passed. This was an environment restriction, not a transport regression; the
failed invocation returned nonzero and removed its temporary state.

## Stage 7 QUIC identity boundary

The current QUIC application exchange authenticates the initiating sender
against the receiver's Pack. It does not return a signed receiver identity
to the initiator, so selected-peer-to-receiver binding cannot be added
without changing the application wire format. Stage 7 leaves this separate
from the encrypted TCP receiver identity fix.
