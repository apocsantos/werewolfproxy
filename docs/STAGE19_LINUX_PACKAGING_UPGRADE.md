# Stage19: Linux packaging, installation, and upgrade safety

## Scope and certified platform

Stage19 packages the existing v1 release candidate without changing the
Stage10 wire protocol, Stage15B persistence format, Stage17 service behavior,
or Stage18 CLI surface. The certified artifact is
`werewolfproxy-1.0.0-rc.1-linux-x86_64` for
`x86_64-unknown-linux-gnu`. The certification environment is Debian GNU/Linux
13 (trixie), x86_64, glibc 2.41, and rustc 1.96.0. Other operating systems,
architectures, container environments, and distributions are not certified by
this stage.

`packaging/release.env` defines the artifact version and target. Cargo package
versions remain unchanged in Stage19; the release version is supplied as fixed
compile-time metadata for `werewolfd --version` and `werewolfctl --version`.
It contains no timestamp or host path.

## Build and artifact

Generate an archive from a clean checkout with:

```sh
SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)" \
  packaging/make-release.sh /absolute/output-directory
```

The script executes:

```sh
cargo build --locked --offline --release --workspace
```

and creates a release directory and deterministic gzip tar archive. It ships
only:

```text
bin/werewolfd
bin/werewolfctl
systemd/werewolfd.service
install.sh
uninstall.sh
INSTALL.md
RELEASE-METADATA
SHA256SUMS
```

`RELEASE-METADATA` records the exact Git commit, artifact version, source date
epoch, rustc version, target triple, Cargo.lock SHA-256, and binary SHA-256.
`SHA256SUMS` covers every distributable file except itself. SHA-256 detects a
locally corrupted artifact; it does not authenticate the publisher. Future
release-signing options include a separately reviewed minisign, GPG, Sigstore,
or platform-attestation process.

Two clean temporary target-directory builds with identical source, rustc,
target, release version, commit metadata, and `SOURCE_DATE_EPOCH` produced
byte-identical `werewolfd` and `werewolfctl` binaries. This is a
characterization of that exact environment, not a claim about every toolchain.

The release builder rejects caller-supplied Rust flags and remaps the checkout
and Cargo registry paths to stable release paths. This prevents builder home
paths from becoming release artifact metadata without stripping symbols.

The release binaries dynamically use the normal Linux runtime loader,
`libc.so.6`, `libm.so.6`, and `libgcc_s.so.1`; the builder does not claim static
linking or compatibility with older glibc versions.

## System installation

Verify the archive before installing:

```sh
tar -xzf werewolfproxy-1.0.0-rc.1-linux-x86_64.tar.gz
cd werewolfproxy-1.0.0-rc.1-linux-x86_64
sha256sum -c SHA256SUMS
sudo ./install.sh
```

The installer uses strict POSIX shell execution, checks the archive hashes,
refuses symlinked sensitive destinations, stages each binary beside its final
pathname, sets its mode and owner, then atomically renames it into place. A
failed candidate therefore leaves the old complete executable or no newly
installed executable; it does not publish a partial binary.

It installs root-owned executable binaries at `/usr/local/bin/werewolfd` and
`/usr/local/bin/werewolfctl`, and the root-owned unit at
`/usr/lib/systemd/system/werewolfd.service`. It creates or validates the
dedicated `werewolf:werewolf` system account and private
`/var/lib/werewolfproxy` Den (`0700`). The systemd unit creates
`/run/werewolfproxy` at boot as `werewolf:werewolf`, `0700`; runtime objects
are not persistent state. No capability is granted to the daemon. The default
high TCP and UDP ports require no root privilege.

The installer only reloads systemd units. It neither enables nor starts the
service, creates Pelt, creates Pack trust, changes Silver, or writes protected
state. Re-running it refreshes software and preserves the Den byte-for-byte.
`DESTDIR=/absolute/staging/root ./install.sh` is supported for rootless package
staging and test inspection.

Provision under the service uid because the local control socket verifies the
client Unix uid:

```sh
sudo -u werewolf werewolfctl --home /var/lib/werewolfproxy den init
sudo systemctl start werewolfd
sudo -u werewolf werewolfctl --socket /run/werewolfproxy/control.sock pelt init
sudo -u werewolf werewolfctl --socket /run/werewolfproxy/control.sock state manifest-migrate
sudo -u werewolf werewolfctl --socket /run/werewolfproxy/control.sock silver off
sudo systemctl enable --now werewolfd
```

Use the Stage18 CLI guide for public identity exchange, Pack, targets, Fangs,
Silver, and doctor. User-mode deployments remain supported with an explicit
private `--home` and runtime control socket; systemd is not required for that
mode.

## Uninstall, backup, upgrade, and downgrade

`sudo ./uninstall.sh` refuses to run while the service is active, removes only
the installed binaries and vendor unit, reloads systemd, and reports that the
Den remains. It never deletes `/var/lib/werewolfproxy`, Pelt, Pack, Silver,
Fangs, target policy, Stage15B selector, or generation files. There is no
automatic purge operation.

Before upgrade, stop the service and make a complete permission-preserving Den
backup in a private location. Pelt private identity is part of that backup:

```sh
sudo systemctl stop werewolfd
sudo install -d -m 0700 /root/werewolf-backup
sudo cp -a /var/lib/werewolfproxy /root/werewolf-backup/
sudo sha256sum -c SHA256SUMS
sudo ./install.sh
sudo systemctl start werewolfd
sudo -u werewolf werewolfctl --socket /run/werewolfproxy/control.sock status
sudo -u werewolf werewolfctl --home /var/lib/werewolfproxy doctor
```

An upgrade safely terminates active forwarding through Stage17 shutdown;
clients reconnect. It does not change the Stage15B format or mutate any
protected state. Pelt, Pack, target policy, Silver, and active-Fang intent are
preserved. In particular, a durably locked Silver state remains locked.
Operators retain a previously verified archive outside the installation if
they want binary rollback. Reinstalling the exact Stage18 binary set is
compatible with state used only by Stage19 because Stage19 has no persistence
change. This is software rollback, not persistent-state anti-rollback.

A single protected-file restore remains rejected by Stage15B. A complete old,
internally consistent Den or machine snapshot can still be accepted without a
non-rollbackable external anchor; Stage19 does not change that boundary.

Future releases must reject unknown newer state schemas, use an explicit
forward migration for schema changes, and document downgrade support before
an irreversible migration. The candidate v1 wire behavior remains frozen
absent a separately reviewed security-critical protocol revision; this stage
adds no negotiation or version disclosure.

## Test evidence and nonclaims

`tests/stage19_release/release.py` verifies archive inventory and checksums,
rootless staging installation, no implicit identity creation, idempotent
reinstall, failed-candidate preservation, unsafe-path rejection, execution
outside the checkout, and uninstall Den preservation. Stage17 lifecycle,
Stage18 provisioning, and acceptance are also executed against the installed
release binaries. The installed acceptance exercises secure TCP and QUIC,
including the existing 50 MiB integrity fixture.

Release artifacts contain no test Dens, Pelt material, PKCS#8, TLS/X25519 or
session secrets, payload captures, debug/test executables, temporary logs, or
developer-home paths. Stage19 makes no publisher-authenticity claim, no
cross-distro glibc claim, no live-upgrade claim, and no claim that installation
or upgrade creates trust automatically.
