#!/usr/bin/env bash
set -euo pipefail

SERVICE_DB="${WEREWOLF_SERVICE_DB:-$HOME/.config/wolf-b/services.json}"

if [[ ! -f "$SERVICE_DB" ]]; then
  echo '{"error":"service database not found"}'
  exit 1
fi

jq '
{
  generated_at: (now | todateiso8601),
  service_count: (keys | length),
  services:
    (
      to_entries
      | sort_by(.key)
      | map({
          name: .key,
          peer: .value.peer,
          target: .value.target,
          kind: .value.kind,
          added_at: .value.added_at,
          last_connected: .value.last_connected
        })
    )
}
' "$SERVICE_DB"
