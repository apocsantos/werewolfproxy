#!/usr/bin/env bash
set -euo pipefail

SOCKET="/tmp/wolf-b.sock"
SERVICE="werewolf-b"

TARGET_URL="${WEREWOLF_TARGET_URL:-http://127.0.0.1:8080}"
QUIC_URL="${WEREWOLF_QUIC_URL:-http://127.0.0.1:9020}"
TCP_URL="${WEREWOLF_TCP_URL:-http://127.0.0.1:9021}"
TCP_ENC_URL="${WEREWOLF_TCP_ENC_URL:-http://127.0.0.1:9022}"

is_healthy() {
  curl --max-time 5 -fsS "$1" >/dev/null
}

measure_latency() {
  local url="$1"
  local out
  if out="$(curl -o /dev/null -s -w "%{time_total}" --max-time 5 "$url")"; then
    echo "$out"
  else
    echo "fail"
  fi
}

choose_json() {
  printf '{"transport":"%s","label":"%s","url":%s,"healthy":%s,"policy":"%s"}\n' \
    "$1" "$2" "$3" "$4" "${5:-secure}"
}

if [[ "${1:-}" == "restart" ]]; then
  echo "🐺 Restarting $SERVICE..."
  systemctl --user restart "$SERVICE"
  sleep 2
  systemctl --user status "$SERVICE" --no-pager
  exit 0
fi

if [[ "${1:-}" == "tail" ]]; then
  exec journalctl --user -u "$SERVICE" -f
fi

if [[ "${1:-}" == "logs" ]]; then
  shift || true
  case "${1:-recent}" in
    errors|--errors) exec journalctl --user -u "$SERVICE" --no-pager | grep -Ei "error|failed|refused|timeout|panic|warn" || true ;;
    quic|--quic) exec journalctl --user -u "$SERVICE" --no-pager | grep -Ei "quic" || true ;;
    tcp|--tcp) exec journalctl --user -u "$SERVICE" --no-pager | grep -Ei "tcp|fang" || true ;;
    *) exec journalctl --user -u "$SERVICE" -n 80 --no-pager ;;
  esac
fi

if [[ "${1:-}" == "transport-health-json" ]]; then
  quic_ok=false; tcp_ok=false; tcp_enc_ok=false
  is_healthy "$QUIC_URL" && quic_ok=true || true
  is_healthy "$TCP_URL" && tcp_ok=true || true
  is_healthy "$TCP_ENC_URL" && tcp_enc_ok=true || true

  jq -n \
    --arg quic_url "$QUIC_URL" \
    --arg tcp_url "$TCP_URL" \
    --arg tcp_enc_url "$TCP_ENC_URL" \
    --argjson quic_ok "$quic_ok" \
    --argjson tcp_ok "$tcp_ok" \
    --argjson tcp_enc_ok "$tcp_enc_ok" \
    '{transports:{quic:{url:$quic_url,healthy:$quic_ok},"tcp-plain":{url:$tcp_url,healthy:$tcp_ok},"tcp-encrypted-v2":{url:$tcp_enc_url,healthy:$tcp_enc_ok}}}'
  exit 0
fi

if [[ "${1:-}" == "health-summary" ]]; then
  echo "🐺 Werewolf Transport Health"
  echo "==========================="
  echo

  data="$("$0" transport-health-json)"
  quic="$(echo "$data" | jq -r '.transports.quic.healthy')"
  tcp_plain="$(echo "$data" | jq -r '.transports["tcp-plain"].healthy')"
  tcp_enc="$(echo "$data" | jq -r '.transports["tcp-encrypted-v2"].healthy')"

  [[ "$quic" == "true" ]] && echo "⚡ QUIC:             healthy ✅" || echo "⚡ QUIC:             failed ❌"
  [[ "$tcp_enc" == "true" ]] && echo "🔐 TCP encrypted v2: healthy ✅" || echo "🔐 TCP encrypted v2: failed ❌"
  [[ "$tcp_plain" == "true" ]] && echo "🦷 TCP plain:        healthy ✅" || echo "🦷 TCP plain:        failed ❌"

  echo
  "$0" auto --json | jq .
  exit 0
fi

if [[ "${1:-}" == "heal" ]]; then
  quiet=0
  [[ "${2:-}" == "--quiet" ]] && quiet=1

  [[ "$quiet" == "0" ]] && echo "🐺 Werewolf Transport Heal" && echo "=========================" && echo

  ensure_profile() {
    local name="$1"
    local port="$2"

    if "$0" fang list | grep -q "local:  127.0.0.1:${port}"; then
      [[ "$quiet" == "0" ]] && echo "✅ $name already active on $port"
    else
      [[ "$quiet" == "0" ]] && echo "🔁 opening $name..."
      "$0" fang open-profile "$name" >/dev/null || true
    fi
  }

  ensure_profile home-web-tcp 9021
  ensure_profile home-web-tcp-encrypted-v2 9022
  ensure_profile large-transfer-tcp-v2 9032
  ensure_profile home-web-quic 9020

  [[ "$quiet" == "0" ]] && echo && "$0" health-summary
  exit 0
fi

if [[ "${1:-}" == "auto-heal-json" ]]; then
  "$0" heal --quiet >/dev/null 2>&1 || true
  "$0" auto --json
  exit $?
fi

if [[ "${1:-}" == "transport-check" ]]; then
  echo "🐺 Werewolf Transport Check (wolf-b)"
  echo "===================================="
  echo
  "$0" health-summary
  exit 0
fi

if [[ "${1:-}" == "fallback-plan" ]]; then
  echo "🐺 Werewolf Fallback Plan (wolf-b)"
  echo "=================================="
  echo
  "$0" health-summary
  exit 0
fi

if [[ "${1:-}" == "status-full" ]]; then
  json_mode=0
  [[ "${2:-}" == "--json" ]] && json_mode=1

  json_or_empty() {
    local out
    if out="$("$@" 2>/dev/null)" && echo "$out" | jq -e . >/dev/null 2>&1; then
      echo "$out"
    else
      echo '{}'
    fi
  }

  if [[ "$json_mode" == "1" ]]; then
    watchdog_active=false
    maintenance_active=false
    systemctl --user is-active --quiet werewolf-b-watchdog.timer && watchdog_active=true || true
    systemctl --user is-active --quiet werewolf-b-maintenance.timer && maintenance_active=true || true

    jq -n \
      --arg timestamp "$(date -Iseconds)" \
      --argjson ready "$(json_or_empty "$0" ready --json)" \
      --argjson doctor "$(json_or_empty "$0" doctor --json)" \
      --argjson scores "$(json_or_empty "$0" score-json)" \
      --argjson health "$(json_or_empty "$0" transport-health-json)" \
      --argjson benchmark "$(json_or_empty "$0" benchmark --json)" \
      --argjson watchdog_active "$watchdog_active" \
      --argjson maintenance_active "$maintenance_active" \
      '{
        timestamp: $timestamp,
        ready: $ready,
        doctor: $doctor,
        scores: $scores,
        health: $health,
        benchmark: $benchmark,
        timers: {
          watchdog_active: $watchdog_active,
          maintenance_active: $maintenance_active
        }
      }'
    exit 0
  fi

  echo "🐺 Werewolf Full Status"
  echo "======================"
  echo

  echo "🧠 Ready"
  "$0" ready --json | jq '.ready, .selected' || true

  echo
  echo "📊 Scores"
  "$0" score || true

  echo
  echo "📜 Policies"
  "$0" policy-test || true

  echo
  echo "🩺 Doctor"
  "$0" doctor || true

  echo
  echo "🛡 Watchdog"
  systemctl --user is-active --quiet werewolf-b-watchdog.timer \
    && echo "watchdog timer: active ✅" \
    || echo "watchdog timer: inactive ❌"

  echo
  echo "🧹 Maintenance"
  systemctl --user is-active --quiet werewolf-b-maintenance.timer \
    && echo "maintenance timer: active ✅" \
    || echo "maintenance timer: inactive ❌"

  echo
  echo "✅ status-full complete"
  exit 0
fi

if [[ "${1:-}" == "maintenance-status" ]]; then
  echo "🐺 Werewolf B Maintenance Status"
  echo "==============================="
  echo

  systemctl --user status werewolf-b-maintenance.timer --no-pager || true
  echo
  systemctl --user status werewolf-b-maintenance.service --no-pager || true
  echo
  journalctl --user -u werewolf-b-maintenance.service -n 40 --no-pager || true

  exit 0
fi

if [[ "${1:-}" == "status-json" ]]; then
  policy="${2:-secure}"

  jq -n \
    --arg timestamp "$(date -Iseconds)" \
    --arg policy "$policy" \
    --argjson ready "$("$0" ready --policy "$policy" --json)" \
    --argjson cache "$("$0" cache-status --json)" \
    --argjson policy_test "$("$0" policy-test | awk '
      BEGIN { print "["; first=1 }
      /^[a-z]/ {
        gsub("->", "", $0)
        policy=$1
        transport=$2
        healthy=$3
        url=$4
        gsub("healthy=", "", healthy)
        gsub("url=", "", url)
        if (!first) print ","
        first=0
        printf("{\"policy\":\"%s\",\"transport\":\"%s\",\"healthy\":%s,\"url\":\"%s\"}", policy, transport, healthy, url)
      }
      END { print "]" }
    ')" \
    '{
      timestamp: $timestamp,
      policy: $policy,
      ready: $ready,
      cache: $cache,
      policy_test: $policy_test
    }'

  exit 0
fi

if [[ "${1:-}" == "cache-status" ]]; then
  json_mode=0
  if [[ "${2:-}" == "--json" ]]; then
    json_mode=1
  fi

  score_file="${WEREWOLF_SCORE_FILE:-$HOME/.cache/werewolf/wolf-b-scores.jsonl}"
  snapshot_dir="${WEREWOLF_SNAPSHOT_DIR:-$HOME/.cache/werewolf/snapshots}"
  report_dir="${WEREWOLF_REPORT_DIR:-$HOME/.cache/werewolf/reports}"

  score_entries=0
  score_size="0"
  [[ -f "$score_file" ]] && score_entries="$(wc -l < "$score_file")" && score_size="$(du -b "$score_file" | awk '{print $1}')"

  snapshot_count="$(find "$snapshot_dir" -maxdepth 1 -name 'wolf-b-*.json' 2>/dev/null | wc -l)"
  snapshot_size="$(du -sb "$snapshot_dir" 2>/dev/null | awk '{print $1}' || echo 0)"

  report_count="$(find "$report_dir" -maxdepth 1 -name 'maintenance-*.json' 2>/dev/null | wc -l)"
  report_size="$(du -sb "$report_dir" 2>/dev/null | awk '{print $1}' || echo 0)"

  if [[ "$json_mode" == "1" ]]; then
    jq -n \
      --arg score_file "$score_file" \
      --arg snapshot_dir "$snapshot_dir" \
      --arg report_dir "$report_dir" \
      --argjson score_entries "$score_entries" \
      --argjson score_size "$score_size" \
      --argjson snapshot_count "$snapshot_count" \
      --argjson snapshot_size "$snapshot_size" \
      --argjson report_count "$report_count" \
      --argjson report_size "$report_size" \
      '{
        scores: { file: $score_file, entries: $score_entries, size_bytes: $score_size },
        snapshots: { dir: $snapshot_dir, count: $snapshot_count, size_bytes: $snapshot_size },
        reports: { dir: $report_dir, count: $report_count, size_bytes: $report_size }
      }'
    exit 0
  fi

  echo "🐺 Werewolf Cache Status"
  echo "======================="
  echo

  if [[ -f "$score_file" ]]; then
    echo "scores:"
    echo "  file:    $score_file"
    echo "  entries: $(wc -l < "$score_file")"
    echo "  size:    $(du -h "$score_file" | awk '{print $1}')"
  else
    echo "scores: none"
  fi

  echo
  echo "snapshots:"
  echo "  dir:     $snapshot_dir"
  echo "  count:   $(find "$snapshot_dir" -maxdepth 1 -name 'wolf-b-*.json' 2>/dev/null | wc -l)"
  echo "  size:    $(du -sh "$snapshot_dir" 2>/dev/null | awk '{print $1}' || echo 0)"

  echo
  echo "reports:"
  echo "  dir:     $report_dir"
  echo "  count:   $(find "$report_dir" -maxdepth 1 -name 'maintenance-*.json' 2>/dev/null | wc -l)"
  echo "  size:    $(du -sh "$report_dir" 2>/dev/null | awk '{print $1}' || echo 0)"

  exit 0
fi

if [[ "${1:-}" == "report-prune" ]]; then
  keep="${2:-30}"
  report_dir="${WEREWOLF_REPORT_DIR:-$HOME/.cache/werewolf/reports}"

  mkdir -p "$report_dir"

  echo "🐺 Werewolf Report Prune"
  echo "======================="
  echo "keeping newest: $keep"
  echo

  mapfile -t old_files < <(ls -1t "$report_dir"/maintenance-*.json 2>/dev/null | tail -n +$((keep + 1)))

  if [[ "${#old_files[@]}" == "0" ]]; then
    echo "✅ nothing to prune"
    exit 0
  fi

  for f in "${old_files[@]}"; do
    echo "🗑 removing $(basename "$f")"
    rm -f "$f"
  done

  echo
  echo "✅ pruned ${#old_files[@]} report(s)"
  exit 0
fi

if [[ "${1:-}" == "maintenance-report" ]]; then
  report_dir="${WEREWOLF_REPORT_DIR:-$HOME/.cache/werewolf/reports}"
  mkdir -p "$report_dir"

  ts="$(date +%Y%m%d_%H%M%S)"
  report="$report_dir/maintenance-${ts}.json"

  echo "🐺 Generating maintenance report..."

  jq -n \
    --arg timestamp "$(date -Iseconds)" \
    --argjson doctor "$("$0" doctor --json)" \
    --argjson ready "$("$0" ready --json)" \
    --argjson benchmark "$("$0" benchmark --json)" \
    --argjson scores "$("$0" score-json)" \
    --argjson history "$("$0" transport-health-json)" \
    '{
      timestamp: $timestamp,
      doctor: $doctor,
      ready: $ready,
      benchmark: $benchmark,
      scores: $scores,
      transport_health: $history
    }' > "$report"

  echo "✅ report saved: $report"
  jq . "$report"

  exit 0
fi

if [[ "${1:-}" == "maintenance" ]]; then
  echo "🐺 Werewolf Maintenance"
  echo "======================"
  echo

  echo "1) cleanup"
  "$0" cleanup

  echo
  echo "2) save score"
  "$0" score-save

  echo
  echo "3) snapshot"
  "$0" snapshot >/tmp/werewolf-maintenance-snapshot.txt
  cat /tmp/werewolf-maintenance-snapshot.txt | head -n 1

  echo
  echo "4) drift alert"
  "$0" snapshot-alert 20 || true

  echo
  echo "5) doctor"
  "$0" doctor

  echo
  echo "✅ maintenance complete"
  exit 0
fi

if [[ "${1:-}" == "cleanup" ]]; then
  keep_snapshots="${WEREWOLF_KEEP_SNAPSHOTS:-50}"
  keep_scores="${WEREWOLF_KEEP_SCORE_SAMPLES:-500}"

  snapshot_dir="${WEREWOLF_SNAPSHOT_DIR:-$HOME/.cache/werewolf/snapshots}"
  score_file="${WEREWOLF_SCORE_FILE:-$HOME/.cache/werewolf/wolf-b-scores.jsonl}"

  echo "🐺 Werewolf Cleanup"
  echo "=================="
  echo
  echo "keep snapshots: $keep_snapshots"
  echo "keep scores:    $keep_scores"
  echo

  mkdir -p "$snapshot_dir"

  if ls "$snapshot_dir"/wolf-b-*.json >/dev/null 2>&1; then
    ls -1t "$snapshot_dir"/wolf-b-*.json | tail -n +"$((keep_snapshots + 1))" | while read -r old; do
      echo "removing snapshot: $(basename "$old")"
      rm -f "$old"
    done
  fi

  if [[ -f "$score_file" ]]; then
    tmp="${score_file}.tmp"
    tail -n "$keep_scores" "$score_file" > "$tmp"
    mv "$tmp" "$score_file"
    echo "score samples kept: $(wc -l < "$score_file")"
  else
    echo "score file: missing"
  fi

  echo
  echo "cache size:"
  du -sh "${HOME}/.cache/werewolf" 2>/dev/null || true

  echo
  echo "✅ cleanup complete"
  exit 0
fi

if [[ "${1:-}" == "snapshot-prune" ]]; then
  keep="${2:-20}"
  snapshot_dir="${WEREWOLF_SNAPSHOT_DIR:-$HOME/.cache/werewolf/snapshots}"

  mkdir -p "$snapshot_dir"

  echo "🐺 Werewolf Snapshot Prune"
  echo "========================="
  echo "keeping newest: $keep"
  echo

  mapfile -t old_files < <(ls -1t "$snapshot_dir"/wolf-b-*.json 2>/dev/null | tail -n +$((keep + 1)))

  if [[ "${#old_files[@]}" == "0" ]]; then
    echo "✅ nothing to prune"
    exit 0
  fi

  for f in "${old_files[@]}"; do
    echo "🗑 removing $(basename "$f")"
    rm -f "$f"
  done

  echo
  echo "✅ pruned ${#old_files[@]} snapshot(s)"
  exit 0
fi

if [[ "${1:-}" == "snapshot-alert" ]]; then
  threshold_ms="${2:-20}"

  diff_json="$("$0" snapshot-diff | sed -n '/^{/,$p')"

  failures=0

  check_delta() {
    local name="$1"
    local jq_path="$2"
    local delta

    delta="$(echo "$diff_json" | jq -r "$jq_path")"

    if awk "BEGIN {exit !($delta > $threshold_ms)}"; then
      echo "⚠ $name latency increased by ${delta}ms > ${threshold_ms}ms"
      failures=$((failures + 1))
    else
      echo "✅ $name latency delta ${delta}ms"
    fi
  }

  echo "🐺 Werewolf Snapshot Alert"
  echo "========================="
  echo "threshold: ${threshold_ms}ms"
  echo

  check_delta "QUIC" '.quic.latency_delta_ms'
  check_delta "TCP encrypted v2" '.tcp_encrypted_v2.latency_delta_ms'
  check_delta "TCP plain" '.tcp_plain.latency_delta_ms'

  echo
  if [[ "$failures" == "0" ]]; then
    echo "🎉 no concerning drift"
    exit 0
  else
    echo "⚠ drift alerts: $failures"
    exit 1
  fi
fi

if [[ "${1:-}" == "snapshot-diff" ]]; then
  snapshot_dir="${WEREWOLF_SNAPSHOT_DIR:-$HOME/.cache/werewolf/snapshots}"

  latest="$(ls -1t "$snapshot_dir"/wolf-b-*.json 2>/dev/null | sed -n '1p')"
  previous="$(ls -1t "$snapshot_dir"/wolf-b-*.json 2>/dev/null | sed -n '2p')"

  if [[ -z "$latest" || -z "$previous" ]]; then
    echo "❌ need at least two snapshots"
    exit 1
  fi

  echo "🐺 Werewolf Snapshot Diff"
  echo "========================"
  echo
  echo "latest:   $(basename "$latest")"
  echo "previous: $(basename "$previous")"
  echo

  jq -n \
    --slurpfile old "$previous" \
    --slurpfile new "$latest" \
    '{
      quic: {
        latency_delta_ms:
          (
            (($new[0].benchmark.transports.quic.latency_seconds // 0)
            -
            ($old[0].benchmark.transports.quic.latency_seconds // 0))
            * 1000
          ),
        score_delta:
          (
            ($new[0].scores.transports.quic.score // 0)
            -
            ($old[0].scores.transports.quic.score // 0)
          )
      },
      tcp_encrypted_v2: {
        latency_delta_ms:
          (
            (($new[0].benchmark.transports["tcp-encrypted-v2"].latency_seconds // 0)
            -
            ($old[0].benchmark.transports["tcp-encrypted-v2"].latency_seconds // 0))
            * 1000
          ),
        score_delta:
          (
            ($new[0].scores.transports["tcp-encrypted-v2"].score // 0)
            -
            ($old[0].scores.transports["tcp-encrypted-v2"].score // 0)
          )
      },
      tcp_plain: {
        latency_delta_ms:
          (
            (($new[0].benchmark.transports["tcp-plain"].latency_seconds // 0)
            -
            ($old[0].benchmark.transports["tcp-plain"].latency_seconds // 0))
            * 1000
          ),
        score_delta:
          (
            ($new[0].scores.transports["tcp-plain"].score // 0)
            -
            ($old[0].scores.transports["tcp-plain"].score // 0)
          )
      }
    }' | jq .

  exit 0
fi

if [[ "${1:-}" == "snapshot" ]]; then
  mkdir -p ~/.cache/werewolf/snapshots
  ts="$(date +%Y%m%d_%H%M%S)"
  out="${WEREWOLF_SNAPSHOT_FILE:-$HOME/.cache/werewolf/snapshots/wolf-b-${ts}.json}"

  "$0" heal --quiet || true
  "$0" score-save >/dev/null 2>&1 || true

  jq -n \
    --arg timestamp "$(date -Iseconds)" \
    --argjson ready "$("$0" ready --json)" \
    --argjson health "$("$0" transport-health-json)" \
    --argjson benchmark "$("$0" benchmark --json)" \
    --argjson scores "$("$0" score-json)" \
    --argjson doctor "$("$0" doctor --json)" \
    '{
      timestamp: $timestamp,
      ready: $ready,
      health: $health,
      benchmark: $benchmark,
      scores: $scores,
      doctor: $doctor
    }' > "$out"

  echo "✅ snapshot saved: $out"
  jq . "$out"

  exit 0
fi

if [[ "${1:-}" == "ready" ]]; then
  json_mode=0
  policy="secure"

  shift || true
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --json)
        json_mode=1
        ;;
      --policy)
        shift || true
        policy="${1:-secure}"
        ;;
      --policy=*)
        policy="${1#--policy=}"
        ;;
      *)
        policy="$1"
        ;;
    esac
    shift || true
  done

  "$0" heal --quiet || true

  if [[ "$json_mode" == "1" ]]; then
    doctor="$("$0" doctor --json)"
    selected="$("$0" auto --policy "$policy" --json)"
    scores="$("$0" score-json)"

    jq -n       --arg policy "$policy"       --argjson doctor "$doctor"       --argjson selected "$selected"       --argjson scores "$scores"       '{
        ready: ($doctor.healthy == true and $selected.healthy == true),
        policy: $policy,
        doctor: $doctor,
        selected: $selected,
        scores: $scores
      }'
    exit 0
  fi

  echo "🐺 Werewolf Ready Check"
  echo "======================"
  echo "policy: $policy"
  echo

  "$0" doctor --json | jq .
  echo

  "$0" policy-explain "$policy"

  exit 0
fi

if [[ "${1:-}" == "doctor-fix" ]]; then
  echo "🐺 Werewolf Doctor Fix"
  echo "====================="
  echo

  echo "🔧 running heal..."
  "$0" heal || true

  echo
  echo "🩺 re-running doctor..."
  "$0" doctor

  exit $?
fi

if [[ "${1:-}" == "doctor" ]]; then
  json_mode=0
  if [[ "${2:-}" == "--json" ]]; then
    json_mode=1
  fi

  if [[ "$json_mode" == "0" ]]; then
    echo "🐺 Werewolf Doctor"
    echo "================="
    echo
  fi

  failures=0
  results="[]"

  check_cmd() {
    local label="$1"
    shift

    if "$@" >/tmp/werewolf-doctor-check.out 2>/tmp/werewolf-doctor-check.err; then
      [[ "$json_mode" == "0" ]] && echo "✅ $label"
      results="$(echo "$results" | jq -c --arg label "$label" '. + [{label:$label, ok:true}]')"
    else
      [[ "$json_mode" == "0" ]] && echo "❌ $label"
      [[ "$json_mode" == "0" ]] && cat /tmp/werewolf-doctor-check.err || true
      err="$(cat /tmp/werewolf-doctor-check.err 2>/dev/null || true)"
      results="$(echo "$results" | jq -c --arg label "$label" --arg err "$err" '. + [{label:$label, ok:false, error:$err}]')"
      failures=$((failures + 1))
    fi
  }

  check_cmd "wolf-b service active" systemctl --user is-active --quiet werewolf-b.service
  check_cmd "wolf-a service active" systemctl --user is-active --quiet werewolf-a.service
  check_cmd "wolf-b socket exists" test -S /tmp/wolf-b.sock
  check_cmd "wolf-a socket exists" test -S /tmp/wolf-a.sock

  check_cmd "transport health json valid" bash -lc "$0 transport-health-json | jq -e '.transports' >/dev/null"
  check_cmd "benchmark json valid" bash -lc "$0 benchmark --json | jq -e '.transports' >/dev/null"
  check_cmd "score json valid" bash -lc "$0 score-json | jq -e '.transports' >/dev/null"
  check_cmd "auto secure valid" bash -lc "$0 auto --policy secure --json | jq -e '.healthy == true' >/dev/null"
  check_cmd "auto resilience valid" bash -lc "$0 auto --policy resilience --json | jq -e '.healthy == true' >/dev/null"
  check_cmd "score history readable" bash -lc "test ! -f ~/.cache/werewolf/wolf-b-scores.jsonl || tail -n 5 ~/.cache/werewolf/wolf-b-scores.jsonl | jq -e . >/dev/null"
  check_cmd "snapshot directory writable" bash -lc "mkdir -p ~/.cache/werewolf/snapshots && test -w ~/.cache/werewolf/snapshots"

  if [[ "$json_mode" == "1" ]]; then
    jq -n --argjson checks "$results" --argjson failures "$failures"       '{healthy:($failures == 0), failures:$failures, checks:$checks}'
  else
    echo
    "$0" policy-test

    echo
    if [[ "$failures" == "0" ]]; then
      echo "🎉 Doctor result: healthy"
    else
      echo "⚠ Doctor result: $failures failure(s)"
    fi
  fi

  [[ "$failures" == "0" ]]
  exit $?
fi

if [[ "${1:-}" == "policy-explain" ]]; then
  policy="${2:-secure}"

  echo "🐺 Werewolf Policy Explain"
  echo "========================="
  echo
  echo "policy: $policy"
  echo

  echo "📊 Current scores"
  "$0" score --no-decision

  echo
  echo "🧠 Decision"
  "$0" auto --policy "$policy" --json | jq .

  echo
  echo "📜 Policy meaning"
  case "$policy" in
    secure)
      echo "secure: prefer QUIC, then TCP encrypted v2, then TCP plain."
      ;;
    performance)
      echo "performance: choose the currently lowest-latency healthy transport."
      ;;
    stealth)
      echo "stealth: prefer TCP encrypted v2, then TCP plain, then QUIC."
      ;;
    resilience)
      echo "resilience: choose the currently highest-scoring healthy transport."
      ;;
    learned)
      echo "learned: choose the historically highest average score, if currently healthy."
      ;;
    recent)
      echo "recent: choose the highest average score from recent score samples."
      ;;
    *)
      echo "unknown policy."
      exit 2
      ;;
  esac

  exit 0
fi

if [[ "${1:-}" == "policy-test" ]]; then
  echo "🐺 Werewolf Policy Test"
  echo "======================"
  echo

  for policy in secure performance stealth resilience learned recent; do
    result="$("$0" auto --policy "$policy" --json || true)"
    transport="$(echo "$result" | jq -r '.transport // "error"')"
    healthy="$(echo "$result" | jq -r '.healthy // false')"
    url="$(echo "$result" | jq -r '.url // "-"')"

    printf "%-12s -> %-18s healthy=%-5s url=%s\n" "$policy" "$transport" "$healthy" "$url"
  done

  exit 0
fi

if [[ "${1:-}" == "score-prune" ]]; then
  keep="${2:-500}"
  score_file="${WEREWOLF_SCORE_FILE:-$HOME/.cache/werewolf/wolf-b-scores.jsonl}"

  if [[ ! -f "$score_file" ]]; then
    echo "✅ no score history to prune"
    exit 0
  fi

  tmp="${score_file}.tmp"
  total="$(wc -l < "$score_file")"

  if [[ "$total" -le "$keep" ]]; then
    echo "✅ score history has $total entries; nothing to prune"
    exit 0
  fi

  tail -n "$keep" "$score_file" > "$tmp"
  mv "$tmp" "$score_file"

  echo "✅ pruned score history: kept $keep of $total entries"
  exit 0
fi

if [[ "${1:-}" == "score-history" ]]; then
  score_file="${WEREWOLF_SCORE_FILE:-$HOME/.cache/werewolf/wolf-b-scores.jsonl}"

  if [[ ! -f "$score_file" ]]; then
    echo "❌ no score history found"
    exit 1
  fi

  echo "🐺 Werewolf Transport Trends"
  echo "============================"
  echo

  analyze_transport() {
    local transport="$1"

    avg_latency="$(jq -s --arg t "$transport" '
      map(.transports[$t].latency_seconds // empty)
      | if length == 0 then null else (add / length) end
    ' "$score_file")"

    avg_score="$(jq -s --arg t "$transport" '
      map(.transports[$t].score // empty)
      | if length == 0 then null else (add / length) end
    ' "$score_file")"

    healthy_pct="$(jq -s --arg t "$transport" '
      map(.transports[$t].healthy)
      | if length == 0 then 0
        else ((map(select(. == true)) | length) / length * 100)
      end
    ' "$score_file")"

    echo "$transport"
    if [[ "$avg_latency" == "null" || -z "$avg_latency" ]]; then
      avg_latency_ms="0"
    else
      avg_latency_ms="$(awk "BEGIN {print $avg_latency * 1000}")"
    fi

    printf "  avg latency:   %.2f ms\n" "$avg_latency_ms"
    printf "  avg score:     %.2f\n" "$(echo "$avg_score" | awk '{print $1+0}')"
    printf "  healthy:       %.1f%%\n" "$(echo "$healthy_pct" | awk '{print $1+0}')"
    echo
  }

  analyze_transport quic
  analyze_transport tcp-encrypted-v2
  analyze_transport tcp-plain

  echo "recommended:"
  "$0" auto --policy resilience --json | jq -r '
    "  " + .label
  '

  exit 0
fi

if [[ "${1:-}" == "score-save" ]]; then
  mkdir -p ~/.cache/werewolf
  score_file="${WEREWOLF_SCORE_FILE:-$HOME/.cache/werewolf/wolf-b-scores.jsonl}"

  "$0" score-json | jq -c --arg ts "$(date -Iseconds)" '. + {timestamp: $ts}' >> "$score_file"

  echo "✅ score saved: $score_file"
  tail -n 1 "$score_file" | jq .

  exit 0
fi

if [[ "${1:-}" == "score" ]]; then
  no_decision=0
  if [[ "${2:-}" == "--no-decision" ]]; then
    no_decision=1
  fi

  echo "🐺 Werewolf Transport Scores"
  echo "==========================="
  echo

  data="$("$0" score-json)"

  for t in quic tcp-encrypted-v2 tcp-plain; do
    healthy="$(echo "$data" | jq -r ".transports[\"$t\"].healthy")"
    latency="$(echo "$data" | jq -r ".transports[\"$t\"].latency_seconds")"
    score="$(echo "$data" | jq -r ".transports[\"$t\"].score")"

    printf "%-18s healthy=%-5s latency=%-10s score=%s\n" "$t" "$healthy" "$latency" "$score"
  done

  if [[ "$no_decision" == "0" ]]; then
    echo
    "$0" auto --policy resilience --json | jq .
  fi

  exit 0
fi

if [[ "${1:-}" == "score-json" ]]; then
  health="$("$0" transport-health-json)"
  bench="$("$0" benchmark --json)"

  jq -n \
    --argjson health "$health" \
    --argjson bench "$bench" \
    '{
      transports: {
        quic: {
          healthy: $health.transports.quic.healthy,
          latency_seconds: $bench.transports.quic.latency_seconds,
          score: (
            if $health.transports.quic.healthy == true then
              100 - (($bench.transports.quic.latency_seconds // 1) * 1000)
            else 0 end
          )
        },
        "tcp-encrypted-v2": {
          healthy: $health.transports["tcp-encrypted-v2"].healthy,
          latency_seconds: $bench.transports["tcp-encrypted-v2"].latency_seconds,
          score: (
            if $health.transports["tcp-encrypted-v2"].healthy == true then
              90 - (($bench.transports["tcp-encrypted-v2"].latency_seconds // 1) * 1000)
            else 0 end
          )
        },
        "tcp-plain": {
          healthy: $health.transports["tcp-plain"].healthy,
          latency_seconds: $bench.transports["tcp-plain"].latency_seconds,
          score: (
            if $health.transports["tcp-plain"].healthy == true then
              70 - (($bench.transports["tcp-plain"].latency_seconds // 1) * 1000)
            else 0 end
          )
        }
      }
    }'

  exit 0
fi

if [[ "${1:-}" == "auto" ]]; then
  json_mode=0
  policy="${WEREWOLF_TRANSPORT_POLICY:-secure}"

  shift || true
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --json) json_mode=1 ;;
      --policy) shift || true; policy="${1:-secure}" ;;
      --policy=*) policy="${1#--policy=}" ;;
    esac
    shift || true
  done

  emit_choice() {
    local transport="$1"
    local label="$2"
    local url="$3"
    if [[ "$json_mode" == "1" ]]; then
      printf '{"transport":"%s","label":"%s","url":"%s","healthy":true,"policy":"%s"}\n' "$transport" "$label" "$url" "$policy"
    else
      echo "transport: $label"
      echo "url:       $url"
    fi
    exit 0
  }

  [[ "$json_mode" == "0" ]] && echo "🐺 Werewolf Auto Transport" && echo "=========================" && echo && echo "policy:    $policy"

  if [[ "$policy" == "recent" ]]; then
    score_file="${WEREWOLF_SCORE_FILE:-$HOME/.cache/werewolf/wolf-b-scores.jsonl}"
    sample_count="${WEREWOLF_SCORE_RECENT_SAMPLES:-20}"

    if [[ -f "$score_file" ]]; then
      best_transport="$(tail -n "$sample_count" "$score_file" | jq -s -r '
        {
          quic: (map(.transports.quic.score // empty) | if length == 0 then -999999 else add / length end),
          "tcp-encrypted-v2": (map(.transports["tcp-encrypted-v2"].score // empty) | if length == 0 then -999999 else add / length end),
          "tcp-plain": (map(.transports["tcp-plain"].score // empty) | if length == 0 then -999999 else add / length end)
        }
        | to_entries
        | sort_by(.value)
        | reverse
        | .[0].key
      ')"

      case "$best_transport" in
        quic)
          is_healthy "$QUIC_URL" && emit_choice "quic" "QUIC 🟢" "$QUIC_URL"
          ;;
        tcp-encrypted-v2)
          is_healthy "$TCP_ENC_URL" && emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL"
          ;;
        tcp-plain)
          is_healthy "$TCP_URL" && emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL"
          ;;
      esac
    fi

    policy="resilience"
  fi

  if [[ "$policy" == "learned" ]]; then
    score_file="${WEREWOLF_SCORE_FILE:-$HOME/.cache/werewolf/wolf-b-scores.jsonl}"

    if [[ -f "$score_file" ]]; then
      best_transport="$(jq -s -r '
        {
          quic: (map(.transports.quic.score // empty) | if length == 0 then -999999 else add / length end),
          "tcp-encrypted-v2": (map(.transports["tcp-encrypted-v2"].score // empty) | if length == 0 then -999999 else add / length end),
          "tcp-plain": (map(.transports["tcp-plain"].score // empty) | if length == 0 then -999999 else add / length end)
        }
        | to_entries
        | sort_by(.value)
        | reverse
        | .[0].key
      ' "$score_file")"

      case "$best_transport" in
        quic)
          is_healthy "$QUIC_URL" && emit_choice "quic" "QUIC 🟢" "$QUIC_URL"
          ;;
        tcp-encrypted-v2)
          is_healthy "$TCP_ENC_URL" && emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL"
          ;;
        tcp-plain)
          is_healthy "$TCP_URL" && emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL"
          ;;
      esac
    fi

    policy="resilience"
  fi

  if [[ "$policy" == "resilience" ]]; then
    data="$("$0" score-json)"

    best_transport=""
    best_score="-999999"

    for t in quic tcp-encrypted-v2 tcp-plain; do
      healthy="$(echo "$data" | jq -r ".transports[\"$t\"].healthy")"
      score="$(echo "$data" | jq -r ".transports[\"$t\"].score")"

      [[ "$healthy" != "true" ]] && continue

      if awk "BEGIN {exit !($score > $best_score)}"; then
        best_score="$score"
        best_transport="$t"
      fi
    done

    case "$best_transport" in
      quic) emit_choice "quic" "QUIC 🟢" "$QUIC_URL" ;;
      tcp-encrypted-v2) emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL" ;;
      tcp-plain) emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL" ;;
    esac
  fi

  if [[ "$policy" == "performance" ]]; then
    data="$("$0" benchmark --json)"
    best_transport=""
    best_latency="999999"

    for t in quic tcp-encrypted-v2 tcp-plain; do
      healthy="$(echo "$data" | jq -r ".transports[\"$t\"].healthy")"
      latency="$(echo "$data" | jq -r ".transports[\"$t\"].latency_seconds")"
      [[ "$healthy" != "true" || "$latency" == "null" ]] && continue

      if awk "BEGIN {exit !($latency < $best_latency)}"; then
        best_latency="$latency"
        best_transport="$t"
      fi
    done

    case "$best_transport" in
      quic) emit_choice "quic" "QUIC 🟢" "$QUIC_URL" ;;
      tcp-encrypted-v2) emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL" ;;
      tcp-plain) emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL" ;;
    esac
  fi

  if [[ "$policy" == "stealth" ]]; then
    is_healthy "$TCP_ENC_URL" && emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL"
    is_healthy "$TCP_URL" && emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL"
    is_healthy "$QUIC_URL" && emit_choice "quic" "QUIC 🟢" "$QUIC_URL"
  fi

  is_healthy "$QUIC_URL" && emit_choice "quic" "QUIC 🟢" "$QUIC_URL"
  is_healthy "$TCP_ENC_URL" && emit_choice "tcp-encrypted-v2" "TCP encrypted v2 🟡" "$TCP_ENC_URL"
  is_healthy "$TCP_URL" && emit_choice "tcp-plain" "TCP plain fallback 🟠" "$TCP_URL"

  if [[ "$json_mode" == "1" ]]; then
    printf '{"transport":"unavailable","label":"unavailable","url":null,"healthy":false,"policy":"%s"}\n' "$policy"
  else
    echo "transport: unavailable 🔴"
  fi
  exit 2
fi

if [[ "${1:-}" == "benchmark" ]]; then
  json_mode=0
  [[ "${2:-}" == "--json" ]] && json_mode=1

  direct="$(measure_latency "$TARGET_URL")"
  quic="$(measure_latency "$QUIC_URL")"
  tcp="$(measure_latency "$TCP_URL")"
  tcp_enc="$(measure_latency "$TCP_ENC_URL")"

  if [[ "$json_mode" == "1" ]]; then
    jq -n \
      --arg target "$TARGET_URL" \
      --arg quic_url "$QUIC_URL" \
      --arg tcp_url "$TCP_URL" \
      --arg tcp_enc_url "$TCP_ENC_URL" \
      --arg direct "$direct" \
      --arg quic "$quic" \
      --arg tcp "$tcp" \
      --arg tcp_enc "$tcp_enc" \
      '{
        target:{url:$target,latency_seconds:(if $direct=="fail" then null else ($direct|tonumber) end),healthy:($direct!="fail")},
        transports:{
          quic:{url:$quic_url,latency_seconds:(if $quic=="fail" then null else ($quic|tonumber) end),healthy:($quic!="fail")},
          "tcp-plain":{url:$tcp_url,latency_seconds:(if $tcp=="fail" then null else ($tcp|tonumber) end),healthy:($tcp!="fail")},
          "tcp-encrypted-v2":{url:$tcp_enc_url,latency_seconds:(if $tcp_enc=="fail" then null else ($tcp_enc|tonumber) end),healthy:($tcp_enc!="fail")}
        }
      }'
    exit 0
  fi

  echo "🐺 Werewolf Benchmark (wolf-b)"
  echo "================================"
  echo
  echo "target: $TARGET_URL"
  echo "quic:   $QUIC_URL"
  echo "tcp:    $TCP_URL"
  echo "tcp-v2: $TCP_ENC_URL"
  echo
  echo "🌐 Direct target latency"
  echo "  direct: $direct s"
  echo
  echo "⚡ QUIC Fang latency"
  echo "  quic:   $quic s"
  echo
  echo "🦷 TCP fallback latency"
  echo "  tcp:    $tcp s"
  echo
  echo "🔐 TCP encrypted v2 latency"
  echo "  tcp-v2: $tcp_enc s"
  exit 0
fi

if [[ "${1:-}" == "watchdog-status" ]]; then
  echo "🐺 Werewolf B Watchdog Status"
  echo "============================"
  echo
  systemctl --user status werewolf-b-watchdog.timer --no-pager || true
  echo
  systemctl --user status werewolf-b-watchdog.service --no-pager || true
  echo
  journalctl --user -u werewolf-b-watchdog.service -n 30 --no-pager || true
  exit 0
fi

exec werewolfctl --socket "$SOCKET" "$@"
