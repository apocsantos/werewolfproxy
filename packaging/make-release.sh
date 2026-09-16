#!/bin/sh
# Build the small, self-contained Linux x86_64 RC archive from a locked tree.
set -eu

fail() { printf '%s\n' "make-release: $*" >&2; exit 1; }
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$ROOT/packaging/release.env"
OUTPUT=${1:-"$ROOT/dist"}
case "$OUTPUT" in /*) ;; *) fail 'output directory must be absolute' ;; esac

commit=$(git -C "$ROOT" rev-parse HEAD) || fail 'Git commit unavailable'
git -C "$ROOT" diff --quiet || fail 'working tree has unstaged changes'
git -C "$ROOT" diff --cached --quiet || fail 'working tree has staged changes'
epoch=${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" show -s --format=%ct HEAD)}
case "$epoch" in *[!0-9]*) fail 'SOURCE_DATE_EPOCH must be numeric' ;; esac
[ -z "${RUSTFLAGS:-}" ] || fail 'RUSTFLAGS must be unset for a certified release build'
[ -z "${CARGO_ENCODED_RUSTFLAGS:-}" ] || fail 'CARGO_ENCODED_RUSTFLAGS must be unset for a certified release build'
cargo_home=${CARGO_HOME:-"$HOME/.cargo"}
case "$cargo_home" in /*) ;; *) fail 'CARGO_HOME must be absolute for a certified release build' ;; esac
release_rustflags="--remap-path-prefix=$ROOT=/usr/src/werewolfproxy --remap-path-prefix=$cargo_home=/usr/local/cargo"

mkdir -p "$OUTPUT"
release_dir="$OUTPUT/werewolfproxy-$RELEASE_VERSION-linux-x86_64"
archive="$OUTPUT/werewolfproxy-$RELEASE_VERSION-linux-x86_64.tar.gz"
[ ! -e "$release_dir" ] || fail "release directory already exists: $release_dir"
[ ! -e "$archive" ] || fail "release archive already exists: $archive"

(cd "$ROOT" && env \
    WEREWOLF_RELEASE_VERSION="$RELEASE_VERSION" \
    WEREWOLF_GIT_COMMIT="$commit" \
    SOURCE_DATE_EPOCH="$epoch" \
    RUSTFLAGS="$release_rustflags" \
    cargo build --locked --offline --release --workspace) || fail 'locked offline release build failed'

mkdir -m 755 "$release_dir"
mkdir -m 755 "$release_dir/bin" "$release_dir/systemd"
install -m 755 "$ROOT/target/release/werewolfd" "$release_dir/bin/werewolfd"
install -m 755 "$ROOT/target/release/werewolfctl" "$release_dir/bin/werewolfctl"
install -m 644 "$ROOT/packaging/systemd/werewolfd.service" "$release_dir/systemd/werewolfd.service"
install -m 755 "$ROOT/packaging/install.sh" "$release_dir/install.sh"
install -m 755 "$ROOT/packaging/uninstall.sh" "$release_dir/uninstall.sh"
install -m 644 "$ROOT/packaging/INSTALL.md" "$release_dir/INSTALL.md"

rustc_version=$(rustc -V | tr '\n' ' ')
target=$(rustc -Vv | sed -n 's/^host: //p')
[ "$target" = "$RELEASE_TARGET" ] || fail "toolchain target $target does not match release target $RELEASE_TARGET"
lock_sha=$(sha256sum "$ROOT/Cargo.lock" | awk '{print $1}')
daemon_sha=$(sha256sum "$release_dir/bin/werewolfd" | awk '{print $1}')
ctl_sha=$(sha256sum "$release_dir/bin/werewolfctl" | awk '{print $1}')
cat > "$release_dir/RELEASE-METADATA" <<EOF
release_version=$RELEASE_VERSION
git_commit=$commit
source_date_epoch=$epoch
rustc=$rustc_version
target=$target
rustflags=$release_rustflags
cargo_lock_sha256=$lock_sha
werewolfd_sha256=$daemon_sha
werewolfctl_sha256=$ctl_sha
EOF

(cd "$release_dir" && sha256sum bin/werewolfd bin/werewolfctl systemd/werewolfd.service install.sh uninstall.sh INSTALL.md RELEASE-METADATA > SHA256SUMS)

parent=$(dirname "$release_dir")
name=$(basename "$release_dir")
(cd "$parent" && tar --sort=name --format=posix --mtime="@$epoch" --owner=0 --group=0 --numeric-owner -cf - "$name" | gzip -n > "$archive") || fail 'archive creation failed'
printf '%s\n' "$release_dir"
printf '%s\n' "$archive"
