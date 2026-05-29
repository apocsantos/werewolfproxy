#!/usr/bin/env bash
set -e

SERVICE="${1:-werewolf-b}"

journalctl --user -u "$SERVICE" -n 100 --no-pager
