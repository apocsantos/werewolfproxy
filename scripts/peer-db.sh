#!/usr/bin/env bash
set -euo pipefail

PEER_DB="${WEREWOLF_PEER_DB:-$HOME/.config/wolf-b/peers.json}"
mkdir -p "$(dirname "$PEER_DB")"

init_db() {
  [[ -f "$PEER_DB" ]] || echo '{}' > "$PEER_DB"
}

case "${1:-}" in
  add)
    init_db
    name="${2:?peer name required}"
    address="${3:?peer address required}"
    shift 3 || true
    probe_cmd="${*:-}"
    peer_id="$(printf "%s|%s" "$name" "$address" | sha256sum | awk '{print $1}')"
    jq \
      --arg name "$name" \
      --arg address "$address" \
      --arg probe_cmd "$probe_cmd" \
      --arg peer_id "$peer_id" \
      --arg ts "$(date -Iseconds)" \
      '.[$name] = {
        id: $peer_id,
        address: $address,
        probe_cmd: (if $probe_cmd == "" then null else $probe_cmd end),
        added_at: $ts,
        last_seen: null,
        healthy: null,
        latency_ms: null,
        score: 0,
        successful_pings: 0,
        failed_pings: 0,
        total_pings: 0,
        availability_pct: 0,
        avg_latency_ms: null,
        reputation: 0
      }' "$PEER_DB" > "$PEER_DB.tmp"
    mv "$PEER_DB.tmp" "$PEER_DB"
    echo "✅ peer added: $name -> $address"
    ;;

  list)
    init_db
    jq . "$PEER_DB"
    ;;

  remove)
    init_db
    name="${2:?peer name required}"
    jq --arg name "$name" 'del(.[$name])' "$PEER_DB" > "$PEER_DB.tmp"
    mv "$PEER_DB.tmp" "$PEER_DB"
    echo "✅ peer removed: $name"
    ;;

  ping)
    init_db
    name="${2:?peer name required}"
    address="$(jq -r --arg name "$name" '.[$name].address // empty' "$PEER_DB")"

    if [[ -z "$address" ]]; then
      echo "❌ unknown peer: $name"
      exit 1
    fi

    probe_cmd="$(jq -r --arg name "$name" '.[$name].probe_cmd // empty' "$PEER_DB")"

    host="${address%:*}"
    port="${address##*:}"

    start_ns="$(date +%s%N)"
    if [[ -n "$probe_cmd" ]]; then
      timeout 5 bash -lc "$probe_cmd" >/dev/null 2>&1
      ok=$?
    else
      timeout 3 bash -lc "cat < /dev/null > /dev/tcp/$host/$port" >/dev/null 2>&1
      ok=$?
    fi

    if [[ "$ok" == "0" ]]; then
      end_ns="$(date +%s%N)"
      latency_ms="$(( (end_ns - start_ns) / 1000000 ))"
      healthy=true
      score=$((100 - latency_ms))
      [[ "$score" -lt 1 ]] && score=1
    else
      latency_ms=null
      healthy=false
      score=0
    fi

    jq \
      --arg name "$name" \
      --arg ts "$(date -Iseconds)" \
      --argjson healthy "$healthy" \
      --argjson latency "$latency_ms" \
      --argjson score "$score" \
      '.[$name].last_seen = $ts
       | .[$name].healthy = $healthy
       | .[$name].latency_ms = $latency
       | .[$name].score = $score
       | .[$name].total_pings = ((.[$name].total_pings // 0) + 1)
       | .[$name].successful_pings = ((.[$name].successful_pings // 0) + (if $healthy then 1 else 0 end))
       | .[$name].failed_pings = ((.[$name].failed_pings // 0) + (if $healthy then 0 else 1 end))
       | .[$name].availability_pct = ((.[$name].successful_pings / .[$name].total_pings) * 100)
       | .[$name].avg_latency_ms =
          (if $healthy and $latency != null then
             (((.[$name].avg_latency_ms // $latency) + $latency) / 2)
           else
             .[$name].avg_latency_ms
           end)
       | .[$name].reputation =
          (((.[$name].availability_pct // 0) * 0.7)
           + ((.[$name].score // 0) * 0.3))' "$PEER_DB" > "$PEER_DB.tmp"
    mv "$PEER_DB.tmp" "$PEER_DB"

    jq --arg name "$name" '.[$name]' "$PEER_DB"
    ;;
  best)
    init_db

    jq '
      to_entries
      | sort_by(.value.reputation // 0)
      | reverse
      | .[]
      | {
          peer: .key,
          reputation: (.value.reputation // 0),
          availability: (.value.availability_pct // 0),
          latency_ms: (.value.avg_latency_ms // null)
        }
    ' "$PEER_DB"
    ;;

  ping-all)
    init_db

    peers="$(jq -r 'keys[]' "$PEER_DB")"

    if [[ -z "$peers" ]]; then
      echo "⚠ no peers in database"
      exit 0
    fi

    while read -r peer; do
      [[ -z "$peer" ]] && continue
      echo "🐾 pinging $peer"
      "$0" ping "$peer"
      echo
    done <<< "$peers"
    ;;

  status)
    init_db

    echo "🐺 Werewolf Peer Status"
    echo "======================"
    echo

    "$0" ping-all >/dev/null || true

    jq -r '
      to_entries
      | sort_by(.value.reputation // 0)
      | reverse
      | .[]
      | "\(.key) | healthy=\(.value.healthy) | reputation=\((.value.reputation // 0) | tostring) | availability=\((.value.availability_pct // 0) | tostring)% | avg_latency=\((.value.avg_latency_ms // "null") | tostring)ms"
    ' "$PEER_DB"
    ;;



  seed-local)
    init_db

    "$0" add wolf-a 127.0.0.1:9560 "wolf-a health"
    "$0" add wolf-b 127.0.0.1:9561 "wolf-b health"
    "$0" add wolf-c 127.0.0.1:9562 "wolf-c health"
    "$0" add wolf-d 127.0.0.1:9563 "wolf-d health"

    "$0" ping-all
    ;;


  route-candidate)
    init_db

    jq '
      to_entries
      | map(select((.value.healthy == true) and ((.value.reputation // 0) > 0)))
      | sort_by(.value.reputation // 0)
      | reverse
      | .[0] // null
      | if . == null then
          {available:false, reason:"no healthy peer"}
        else
          {
            available:true,
            peer:.key,
            address:.value.address,
            reputation:(.value.reputation // 0),
            availability_pct:(.value.availability_pct // 0),
            avg_latency_ms:(.value.avg_latency_ms // null)
          }
        end
    ' "$PEER_DB"
    ;;


  route-explain)
    init_db

    candidate="$("$0" route-candidate)"

    echo "🐺 Werewolf Peer Route Explain"
    echo "============================="
    echo

    if echo "$candidate" | jq -e '.available == true' >/dev/null; then
      echo "selected:      $(echo "$candidate" | jq -r '.peer')"
      echo "address:       $(echo "$candidate" | jq -r '.address')"
      echo "reputation:    $(echo "$candidate" | jq -r '.reputation')"
      echo "availability:  $(echo "$candidate" | jq -r '.availability_pct')%"
      echo "avg latency:   $(echo "$candidate" | jq -r '.avg_latency_ms')ms"
      echo
      echo "reason: highest healthy peer reputation"
    else
      echo "no route candidate available"
      echo "$candidate" | jq .
      exit 1
    fi
    ;;


  id)
    init_db
    name="${2:?peer name required}"
    jq --arg name "$name" '.[$name] | {name:$name, id:.id, address:.address}' "$PEER_DB"
    ;;

  distance)
    init_db
    a="${2:?first peer required}"
    b="${3:?second peer required}"

    id_a="$(jq -r --arg name "$a" '.[$name].id // empty' "$PEER_DB")"
    id_b="$(jq -r --arg name "$b" '.[$name].id // empty' "$PEER_DB")"

    if [[ -z "$id_a" || -z "$id_b" ]]; then
      echo "❌ unknown peer id"
      exit 1
    fi

    # Simple KAD-style approximate distance:
    # count equal leading hex chars, then distance bucket = 64 - common_prefix.
    prefix=0
    for i in $(seq 0 63); do
      ca="${id_a:$i:1}"
      cb="${id_b:$i:1}"
      if [[ "$ca" == "$cb" ]]; then
        prefix=$((prefix + 1))
      else
        break
      fi
    done

    bucket=$((64 - prefix))

    jq -n \
      --arg a "$a" \
      --arg b "$b" \
      --arg id_a "$id_a" \
      --arg id_b "$id_b" \
      --argjson common_prefix "$prefix" \
      --argjson bucket "$bucket" \
      '{
        a:$a,
        b:$b,
        id_a:$id_a,
        id_b:$id_b,
        common_prefix_hex:$common_prefix,
        distance_bucket:$bucket
      }'
    ;;


  nearest)
    init_db
    target="${2:?target peer required}"

    target_id="$(jq -r --arg name "$target" '.[$name].id // empty' "$PEER_DB")"

    if [[ -z "$target_id" ]]; then
      echo "❌ unknown target peer: $target"
      exit 1
    fi

    jq -r --arg target "$target" 'keys[] | select(. != $target)' "$PEER_DB" | while read -r peer; do
      peer_id="$(jq -r --arg name "$peer" '.[$name].id // empty' "$PEER_DB")"

      prefix=0
      for i in $(seq 0 63); do
        ca="${target_id:$i:1}"
        cb="${peer_id:$i:1}"
        if [[ "$ca" == "$cb" ]]; then
          prefix=$((prefix + 1))
        else
          break
        fi
      done

      bucket=$((64 - prefix))
      reputation="$(jq -r --arg name "$peer" '.[$name].reputation // 0' "$PEER_DB")"
      healthy="$(jq -r --arg name "$peer" '.[$name].healthy // false' "$PEER_DB")"
      address="$(jq -r --arg name "$peer" '.[$name].address // ""' "$PEER_DB")"

      jq -n \
        --arg peer "$peer" \
        --arg address "$address" \
        --argjson bucket "$bucket" \
        --argjson reputation "$reputation" \
        --argjson healthy "$healthy" \
        '{peer:$peer,address:$address,distance_bucket:$bucket,reputation:$reputation,healthy:$healthy}'
    done | jq -s 'sort_by(.distance_bucket, -(.reputation // 0))'
    ;;


  bucket)
    init_db
    target="${2:?target peer required}"

    "$0" nearest "$target" \
      | jq '
          group_by(.distance_bucket)
          | map({
              bucket: .[0].distance_bucket,
              peers: sort_by(-(.reputation // 0))
            })
          | sort_by(.bucket)
        '
    ;;

  *)
    cat <<HELP
🐺 Werewolf Peer DB

Usage:
  scripts/peer-db.sh add <name> <host:port> [probe_cmd...]
  scripts/peer-db.sh list
  scripts/peer-db.sh ping <name>
  scripts/peer-db.sh ping-all
  scripts/peer-db.sh seed-local
  scripts/peer-db.sh status
  scripts/peer-db.sh best
  scripts/peer-db.sh route-candidate
  scripts/peer-db.sh route-explain
  scripts/peer-db.sh id <name>
  scripts/peer-db.sh distance <peer-a> <peer-b>
  scripts/peer-db.sh nearest <peer>
  scripts/peer-db.sh bucket <peer>
  scripts/peer-db.sh remove <name>

DB:
  $PEER_DB
HELP
    ;;
esac
