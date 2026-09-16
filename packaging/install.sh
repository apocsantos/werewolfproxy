#!/bin/sh
# Install a verified Stage19 archive. This script intentionally does not start
# the daemon, mint Pelt, or create trust state.
set -eu

fail() { printf '%s\n' "install: $*" >&2; exit 1; }
note() { printf '%s\n' "$*"; }

SOURCE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
DESTDIR=${DESTDIR:-}

case "$DESTDIR" in
    '') ROOT= ;;
    /*) ROOT=$DESTDIR ;;
    *) fail 'DESTDIR must be absolute' ;;
esac

# A staging root is an installation boundary too; never traverse a caller-
# supplied symlink before creating system-layout descendants beneath it.
if [ -n "$ROOT" ]; then
    if [ -e "$ROOT" ]; then
        [ ! -L "$ROOT" ] && [ -d "$ROOT" ] || fail "unsafe DESTDIR: $ROOT"
    else
        mkdir -m 755 "$ROOT" || fail "cannot create DESTDIR: $ROOT"
    fi
fi

[ -f "$SOURCE_DIR/SHA256SUMS" ] || fail 'missing SHA256SUMS'
(cd "$SOURCE_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail 'release checksum verification failed'

if [ -z "$ROOT" ] && [ "$(id -u)" -ne 0 ]; then
    fail 'system installation requires root; use absolute DESTDIR for staging'
fi

ensure_dir() {
    path=$1 mode=$2
    if [ -L "$path" ]; then fail "refusing symlink directory: $path"; fi
    if [ -e "$path" ]; then
        [ -d "$path" ] || fail "expected directory: $path"
    else
        mkdir -m "$mode" "$path" || fail "cannot create directory: $path"
    fi
}

require_regular_destination() {
    path=$1
    [ ! -L "$path" ] || fail "refusing symlink destination: $path"
    if [ -e "$path" ]; then [ -f "$path" ] || fail "expected regular file: $path"; fi
}

atomic_install() {
    source=$1 destination=$2 mode=$3
    require_regular_destination "$destination"
    temporary=$(mktemp "${destination}.new.XXXXXX") || fail "cannot stage $destination"
    trap 'rm -f "$temporary"' HUP INT TERM EXIT
    cat "$source" > "$temporary" || fail "cannot copy $source"
    chmod "$mode" "$temporary" || fail "cannot set mode on $temporary"
    if [ -z "$ROOT" ]; then chown root:root "$temporary" || fail "cannot set owner on $temporary"; fi
    mv -f "$temporary" "$destination" || fail "cannot publish $destination"
    trap - HUP INT TERM EXIT
}

ensure_dir "${ROOT}/usr" 755
ensure_dir "${ROOT}/usr/local" 755
ensure_dir "${ROOT}/usr/bin" 755
ensure_dir "${ROOT}/usr/lib" 755
ensure_dir "${ROOT}/usr/lib/systemd" 755
ensure_dir "${ROOT}/usr/lib/systemd/system" 755
ensure_dir "${ROOT}/var" 755
ensure_dir "${ROOT}/var/lib" 755

if [ -z "$ROOT" ]; then
    if getent group werewolf >/dev/null 2>&1; then
        :
    else
        groupadd --system werewolf || fail 'cannot create werewolf group'
    fi
    if id -u werewolf >/dev/null 2>&1; then
        [ "$(id -gn werewolf)" = werewolf ] || fail 'existing werewolf account has unexpected primary group'
    else
        useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin --gid werewolf werewolf || fail 'cannot create werewolf user'
    fi
fi

state_dir="${ROOT}/var/lib/werewolfproxy"
if [ -e "$state_dir" ] || [ -L "$state_dir" ]; then
    [ ! -L "$state_dir" ] && [ -d "$state_dir" ] || fail "unsafe state directory: $state_dir"
    [ "$(stat -c '%a' "$state_dir")" = 700 ] || fail 'existing state directory must have mode 0700'
    if [ -z "$ROOT" ]; then
        [ "$(stat -c '%U:%G' "$state_dir")" = 'werewolf:werewolf' ] || fail 'existing state directory has unexpected owner'
    fi
else
    mkdir -m 700 "$state_dir" || fail 'cannot create state directory'
    if [ -z "$ROOT" ]; then chown werewolf:werewolf "$state_dir" || fail 'cannot set state directory owner'; fi
fi

atomic_install "$SOURCE_DIR/bin/werewolfd" "${ROOT}/usr/local/bin/werewolfd" 755
atomic_install "$SOURCE_DIR/bin/werewolfctl" "${ROOT}/usr/local/bin/werewolfctl" 755
atomic_install "$SOURCE_DIR/systemd/werewolfd.service" "${ROOT}/usr/lib/systemd/system/werewolfd.service" 644

if [ -z "$ROOT" ] && command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || fail 'systemctl daemon-reload failed'
fi

note 'Installed WerewolfProxy software.'
note 'Persistent Den preserved or created at /var/lib/werewolfproxy.'
note 'Identity was NOT created. Provision Pelt and Pack explicitly before enabling the service.'
