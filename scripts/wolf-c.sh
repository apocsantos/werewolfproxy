#!/usr/bin/env bash
exec werewolfctl --socket "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is required}/werewolf-c/control.sock" "$@"
