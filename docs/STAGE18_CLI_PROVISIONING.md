# Stage18: CLI provisioning and operational UX

## Scope

Stage18 makes the supported Linux administrative workflows available through
`werewolfctl`. It changes neither a network transcript nor the Stage15B
storage format. Administrative mutations continue to enter the daemon through
the bounded, same-UID control socket and its single mutation coordinator.

The original control surface had `pelt init` and fingerprint output, Pack add,
remove, revoke, address update, and list commands; whole-document target
policy replacement; transient/profile Fang commands; Silver trigger/reset;
status, health, diagnose, Den info, and manifest migration. It lacked a Den
bootstrap command, public identity export, public-key-first Pack provisioning,
incremental exact target operations, profile-level deactivation, offline
diagnosis, and a backup inventory. It also had no supported JSON-output or
exit-class contract.

## Command surface

The Stage18 v1 commands are:

```text
werewolfctl [--socket PATH] [--home PATH] [--json] den init|info
werewolfctl ... pelt init|show
werewolfctl ... pack add NAME PUBLIC_KEY_B64 ADDRESS
werewolfctl ... pack list|remove NAME|revoke NAME|set-address NAME ADDRESS
werewolfctl ... target allow PEER IP:PORT|list|remove PEER IP:PORT
werewolfctl ... fang create NAME PEER LOCAL REMOTE [--transport quic|tcp|tcp-plain]
werewolfctl ... fang list|active|activate NAME|deactivate NAME|remove NAME
werewolfctl ... silver status|on|off
werewolfctl ... status|health|diagnose|doctor
werewolfctl ... state manifest-migrate|backup-info
```

`--socket` selects an explicit daemon control socket and takes precedence over
the normal private runtime socket. `--home` is used by offline commands such
as `den init`, `doctor`, and `state backup-info`; online mutations target the
Den owned by the selected daemon. A daemon is started with its matching
explicit `--home` and, when necessary, `--socket`; no CLI command silently
selects a different Den.

Hidden compatibility commands retain the former whole-policy and low-level
Fang operations for existing local scripts, but normal v1 administration uses
the object/verb commands above. `--help` documents every visible command.

## Bootstrap and peer provisioning

Create a private Den without generating an identity:

```bash
werewolfctl --home /var/lib/werewolfproxy den init
werewolfd --home /var/lib/werewolfproxy --socket /run/werewolfproxy/control.sock
werewolfctl --socket /run/werewolfproxy/control.sock pelt init
werewolfctl --socket /run/werewolfproxy/control.sock --json pelt show
```

`pelt init` is create-only. Repeating it returns a local rejection and never
rotates Pelt. `pelt show` returns the fingerprint and `public_key_b64` needed
by an administrator provisioning another machine; it never returns a seed,
private key, PKCS#8 material, TLS private material, or session material.

Exchange the two public `pelt show --json` results out of band, then add each
peer using its public key and a transport address, for example:

```bash
werewolfctl --socket /run/werewolfproxy/control.sock \
  pack add edge-b 'BASE64_ED25519_PUBLIC_KEY' tcp://203.0.113.10:8443
```

The CLI derives the fingerprint from the supplied public key. The daemon
rechecks that relation, validates the Pack, rejects duplicate/conflicting
identities, and durably commits before publishing the candidate Pack. There is
no trust-on-first-use path and no command asks for peer private material.
`pack remove` and `pack revoke` retain the Stage11C order: authority deny fence,
durable protected-state commit, then final runtime publication and cleanup.

## Policy and Fang administration

Target policy changes are exact literal IP-address and port grants only:

```bash
werewolfctl --socket /run/werewolfproxy/control.sock \
  target allow wwp1:AA-BB-CC-DD-EE-FF-00-11 127.0.0.1:8080
werewolfctl --socket /run/werewolfproxy/control.sock target list
werewolfctl --socket /run/werewolfproxy/control.sock \
  target remove wwp1:AA-BB-CC-DD-EE-FF-00-11 127.0.0.1:8080
```

The daemon resolves a Pack name or fingerprint to the Pack fingerprint, parses
an exact `SocketAddr`, and creates a candidate deny-by-default policy. It
rejects hostnames, CIDR, ranges, wildcards, scoped IPv6, port zero, and a
legacy-allow policy rather than silently changing its meaning. Candidate
validation, Stage15B generation commit, and runtime publication remain one
serialized daemon transaction.

Create and operate a persistent Fang profile without JSON editing:

```bash
werewolfctl --socket /run/werewolfproxy/control.sock \
  fang create mail edge-b 127.0.0.1:15432 127.0.0.1:5432 --transport tcp
werewolfctl --socket /run/werewolfproxy/control.sock fang activate mail
werewolfctl --socket /run/werewolfproxy/control.sock fang active
werewolfctl --socket /run/werewolfproxy/control.sock fang deactivate mail
werewolfctl --socket /run/werewolfproxy/control.sock fang remove mail
```

Activation durably records the active-profile intent before the listener is
published. Deactivation durably removes that intent before cancelling matching
runtime Fangs, so a restart cannot resurrect the profile. Removing an active
profile is rejected; deactivate it first. Profile creation accepts only the
existing `quic`, `tcp`, and explicit `tcp-plain` transport values. `STRICT` is
the normal secure choice; compatibility and legacy/plain behavior remain
explicit local choices and must be treated as downgrade-sensitive operational
profiles, never as a change in cryptographic trust.

## Silver, status, JSON, and errors

`silver status`, `silver on`, and `silver off` map to the certified strict
latch operations. Silver on fences authority before persistence; Silver off
does not publish unlocked authority until durable protected-state reconciliation
succeeds. Status is local and reports daemon reachability, `READY` or `LOCKED`
lifecycle, Silver state, Pelt state and public fingerprint, loaded protected
generation, configured/running Fang counts, configured TCP/QUIC endpoints, and
whether secure listeners are configured or waiting for Pelt.

Use `--json` for automation. It emits exactly one JSON object on stdout with
stable Stage18 fields `ok`, `command`, and `result`; diagnostics are sent to
stderr. The broad exit classes are success (0), usage/input (64), local
security-state (65), conflict/already-running (66), daemon unavailable (69),
and locally rejected operation (70). No normal CLI output includes secret
material.

`doctor` is read-only. It validates the Den boundary, detects malformed or
incomplete Stage15B selector state, reports legacy manifest status, and checks
Pelt validation when present. It never repairs a Den. A stopped-Den doctor
report is deliberately conservative: startup remains the full manifest
commitment validator and still fails closed for an inconsistent generation.

## Migration, backup, restore, and service use

Run `state manifest-migrate` explicitly against a running daemon after it has
validated legacy state. It is one-way for normal operation: once manifest mode
exists, missing or damaged selector state is not silently treated as legacy.
Repeating migration reports that it is already enabled.

`state backup-info` lists the persistent Pelt and both Stage15B generation
slots, selector, marker, and manifests. For v1, stop the daemon before copying
the complete Den; do not copy control sockets, lock files, or runtime
directories. After restore run `werewolfctl --home PATH doctor`, then start the
daemon so its startup validation completes before forwarding. A complete old
internally consistent Den remains acceptable without an external monotonic
anchor, exactly as documented by Stages15A/15B.

For the Stage17 service layout use `--home /var/lib/werewolfproxy` and its
configured `/run/werewolfproxy/control.sock`; lifecycle remains with
`systemctl start|stop|restart|status werewolfd`. `werewolfctl` does not shell
out to systemd and remains supported for foreground user-mode deployments.
A real transient systemd service ran the same bootstrap, Pelt initialization,
manifest migration, and status workflow as `nobody:nogroup` using systemd
StateDirectory/RuntimeDirectory provisioning. It exited successfully. The
temporary service state and copied install-like binaries were removed after
the check.

## Validation and security boundary

The Stage18 disposable harness starts two fresh user-mode daemons, initializes
Pelt, exchanges only public descriptors, migrates state, unlocks Silver,
provisions Pack and exact target policy, creates and activates a TCP Fang,
passes data through it, proves activation intent survives restart and
deactivation does not, races target mutations without losing an update,
exercises Silver and Pack/target revocation, and verifies doctor detects a
stopped-Den malformed selector. It uses no normal direct JSON edits; the one
direct corruption is a disposable negative diagnostic fixture.

All protected mutations use the existing Stage15B candidate -> validate ->
durable generation commit -> runtime publication path, except the already
certified Silver-on authority fence that must precede persistence. Stage18
does not change Stage9 authorization, Stage10 framing/KDF/replay, Stage11A
fallback, Stage11C authority, Stage12 receiver authentication, Stage13B write
coalescing, Stage14 secret handling, Stage16 admission limits, or Stage17
lifecycle. It adds no protocol messages, no wire-visible CLI operations, no
persistence schema, and no dependency.
