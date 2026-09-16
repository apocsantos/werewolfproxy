#!/bin/sh
# Remove Stage19 software without destroying persistent security state.
set -eu

fail() { printf '%s\n' "uninstall: $*" >&2; exit 1; }
note() { printf '%s\n' "$*"; }
DESTDIR=${DESTDIR:-}
case "$DESTDIR" in '') ROOT= ;; /*) ROOT=$DESTDIR ;; *) fail 'DESTDIR must be absolute' ;; esac

if [ -z "$ROOT" ] && [ "$(id -u)" -ne 0 ]; then fail 'system uninstall requires root'; fi
if [ -z "$ROOT" ] && command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet werewolfd; then
    fail 'werewolfd is active; stop and disable it before uninstalling software'
fi

remove_regular() {
    path=$1
    [ ! -L "$path" ] || fail "refusing symlink destination: $path"
    if [ -e "$path" ]; then [ -f "$path" ] || fail "expected regular file: $path"; rm -f "$path"; fi
}
remove_regular "${ROOT}/usr/local/bin/werewolfd"
remove_regular "${ROOT}/usr/local/bin/werewolfctl"
remove_regular "${ROOT}/usr/lib/systemd/system/werewolfd.service"
if [ -z "$ROOT" ] && command -v systemctl >/dev/null 2>&1; then systemctl daemon-reload || fail 'systemctl daemon-reload failed'; fi
note 'WerewolfProxy software removed.'
note 'Persistent Den retained at /var/lib/werewolfproxy.'
