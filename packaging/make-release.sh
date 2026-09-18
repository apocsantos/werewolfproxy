#!/bin/sh
# Build the small, self-contained Linux x86_64 RC archive from a locked tree.
set -eu

fail() { printf '%s\n' "make-release: $*" >&2; exit 1; }
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$ROOT/packaging/release.env"
OUTPUT=${1:-"$ROOT/dist"}
case "$OUTPUT" in /*) ;; *) fail 'output directory must be absolute' ;; esac
TARGET_DIR=${CARGO_TARGET_DIR:-"$ROOT/target"}
case "$TARGET_DIR" in /*) ;; *) fail 'CARGO_TARGET_DIR must be absolute' ;; esac

commit=$(git -C "$ROOT" rev-parse HEAD) || fail 'Git commit unavailable'
git -C "$ROOT" diff --quiet || fail 'working tree has unstaged changes'
git -C "$ROOT" diff --cached --quiet || fail 'working tree has staged changes'
git -C "$ROOT" merge-base --is-ancestor "$PRODUCTION_CODE_COMMIT" "$commit" || fail 'release source does not contain the certified production-code commit'
epoch=${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" show -s --format=%ct HEAD)}
case "$epoch" in *[!0-9]*) fail 'SOURCE_DATE_EPOCH must be numeric' ;; esac
[ -z "${RUSTFLAGS:-}" ] || fail 'RUSTFLAGS must be unset for a certified release build'
[ -z "${CARGO_ENCODED_RUSTFLAGS:-}" ] || fail 'CARGO_ENCODED_RUSTFLAGS must be unset for a certified release build'
cargo_home=${CARGO_HOME:-"$HOME/.cargo"}
case "$cargo_home" in /*) ;; *) fail 'CARGO_HOME must be absolute for a certified release build' ;; esac
release_rustflags="--remap-path-prefix=$ROOT=/usr/src/werewolfproxy --remap-path-prefix=$cargo_home=/usr/local/cargo"

mkdir -p "$OUTPUT"
mkdir -p "$TARGET_DIR"
release_dir="$OUTPUT/werewolfproxy-$RELEASE_VERSION-linux-x86_64"
archive="$OUTPUT/werewolfproxy-$RELEASE_VERSION-linux-x86_64.tar.gz"
[ ! -e "$release_dir" ] || fail "release directory already exists: $release_dir"
[ ! -e "$archive" ] || fail "release archive already exists: $archive"
[ ! -e "$OUTPUT/SHA256SUMS" ] && [ ! -L "$OUTPUT/SHA256SUMS" ] || fail "refusing existing outer checksum path: $OUTPUT/SHA256SUMS"

(cd "$ROOT" && env \
    WEREWOLF_RELEASE_VERSION="$RELEASE_VERSION" \
    WEREWOLF_GIT_COMMIT="$PRODUCTION_CODE_COMMIT" \
    SOURCE_DATE_EPOCH="$epoch" \
    CARGO_TARGET_DIR="$TARGET_DIR" \
    RUSTFLAGS="$release_rustflags" \
    cargo build --locked --offline --release --workspace) || fail 'locked offline release build failed'

mkdir -m 755 "$release_dir"
mkdir -m 755 "$release_dir/bin" "$release_dir/systemd" "$release_dir/docs"
install -m 755 "$TARGET_DIR/release/werewolfd" "$release_dir/bin/werewolfd"
install -m 755 "$TARGET_DIR/release/werewolfctl" "$release_dir/bin/werewolfctl"
install -m 644 "$ROOT/packaging/systemd/werewolfd.service" "$release_dir/systemd/werewolfd.service"
install -m 755 "$ROOT/packaging/install.sh" "$release_dir/install.sh"
install -m 755 "$ROOT/packaging/uninstall.sh" "$release_dir/uninstall.sh"
install -m 644 "$ROOT/packaging/INSTALL.md" "$release_dir/INSTALL.md"
install -m 644 "$ROOT/README.md" "$release_dir/README.md"
install -m 644 "$ROOT/QUICKSTART.md" "$release_dir/QUICKSTART.md"
install -m 644 "$ROOT/SECURITY.md" "$release_dir/SECURITY.md"
install -m 644 "$ROOT/RELEASE_NOTES_1.0.0-rc.1.md" "$release_dir/RELEASE_NOTES.md"
install -m 644 "$ROOT/packaging/SBOM.json" "$release_dir/SBOM.json"
install -m 644 "$ROOT/packaging/THIRD_PARTY_NOTICES" "$release_dir/THIRD_PARTY_NOTICES"
install -m 644 "$ROOT/docs/STAGE20S_RUSTLS_2026_0285_REMEDIATION.md" "$release_dir/docs/STAGE20S_RUSTLS_2026_0285_REMEDIATION.md"

rustc_version=$(rustc -V | tr '\n' ' ')
target=$(rustc -Vv | sed -n 's/^host: //p')
[ "$target" = "$RELEASE_TARGET" ] || fail "toolchain target $target does not match release target $RELEASE_TARGET"
libc_version=$(ldd --version 2>&1 | sed -n '1p')
lock_sha=$(sha256sum "$ROOT/Cargo.lock" | awk '{print $1}')
rustls_version=$(python3 -c 'import pathlib,sys,tomllib; packages=tomllib.loads(pathlib.Path(sys.argv[1]).read_text())["package"]; print(next(p["version"] for p in packages if p["name"] == "rustls"))' "$ROOT/Cargo.lock")
webpki_version=$(python3 -c 'import pathlib,sys,tomllib; packages=tomllib.loads(pathlib.Path(sys.argv[1]).read_text())["package"]; print(next(p["version"] for p in packages if p["name"] == "rustls-webpki"))' "$ROOT/Cargo.lock")
[ "$rustls_version" = 0.23.45 ] || fail "unexpected rustls version: $rustls_version"
[ "$webpki_version" = 0.103.15 ] || fail "unexpected rustls-webpki version: $webpki_version"
grep -F "\"cargo_lock_sha256\": \"$lock_sha\"" "$ROOT/packaging/SBOM.json" >/dev/null || fail 'SBOM does not match Cargo.lock'
grep -F '"name": "rustls"' "$ROOT/packaging/SBOM.json" >/dev/null || fail 'SBOM does not list rustls'
grep -F '"version": "0.23.45"' "$ROOT/packaging/SBOM.json" >/dev/null || fail 'SBOM does not resolve patched rustls'
grep -F '"version": "0.103.15"' "$ROOT/packaging/SBOM.json" >/dev/null || fail 'SBOM does not resolve patched rustls-webpki'
daemon_sha=$(sha256sum "$release_dir/bin/werewolfd" | awk '{print $1}')
ctl_sha=$(sha256sum "$release_dir/bin/werewolfctl" | awk '{print $1}')
cat > "$release_dir/RELEASE-METADATA" <<EOF
release_version=$RELEASE_VERSION
git_commit=$commit
production_code_commit=$PRODUCTION_CODE_COMMIT
source_date_epoch=$epoch
rustc=$rustc_version
target=$target
rustflags=--remap-path-prefix=<checkout>=/usr/src/werewolfproxy --remap-path-prefix=<cargo-home>=/usr/local/cargo
cargo_lock_sha256=$lock_sha
rustls=$rustls_version
rustls_webpki=$webpki_version
werewolfd_sha256=$daemon_sha
werewolfctl_sha256=$ctl_sha
EOF

cat > "$release_dir/RELEASE-MANIFEST.json" <<EOF
{
  "name": "WerewolfProxy",
  "version": "$RELEASE_VERSION",
  "git_commit": "$commit",
  "production_code_commit": "$PRODUCTION_CODE_COMMIT",
  "rustc": "$rustc_version",
  "target": "$target",
  "libc_test_environment": "Debian GNU/Linux 13.6; glibc 2.41 ($libc_version)",
  "cargo_lock_sha256": "$lock_sha",
  "dependency_versions": {"rustls": "$rustls_version", "rustls-webpki": "$webpki_version"},
  "source_date_epoch": $epoch,
  "artifact_names": ["bin/werewolfd", "bin/werewolfctl", "systemd/werewolfd.service"],
  "artifact_sha256": {
    "bin/werewolfd": "$daemon_sha",
    "bin/werewolfctl": "$ctl_sha",
    "systemd/werewolfd.service": "$(sha256sum "$release_dir/systemd/werewolfd.service" | awk '{print $1}')"
  },
  "certification_status": "Stage21 RC1 audit candidate",
  "platform_scope": ["Linux x86_64", "x86_64-unknown-linux-gnu"]
}
EOF

(cd "$release_dir" && sha256sum bin/werewolfd bin/werewolfctl systemd/werewolfd.service install.sh uninstall.sh INSTALL.md README.md QUICKSTART.md SECURITY.md RELEASE_NOTES.md SBOM.json THIRD_PARTY_NOTICES docs/STAGE20S_RUSTLS_2026_0285_REMEDIATION.md RELEASE-METADATA RELEASE-MANIFEST.json > SHA256SUMS)

parent=$(dirname "$release_dir")
name=$(basename "$release_dir")
(cd "$parent" && tar --sort=name --format=posix --pax-option=delete=atime,delete=ctime --mtime="@$epoch" --owner=0 --group=0 --numeric-owner -cf - "$name" | gzip -n > "$archive") || fail 'archive creation failed'
printf '%s\n' "$release_dir"
printf '%s\n' "$archive"
archive_sha=$(sha256sum "$archive" | awk '{print $1}')
printf '%s  %s\n' "$archive_sha" "$name.tar.gz" > "$OUTPUT/SHA256SUMS"
