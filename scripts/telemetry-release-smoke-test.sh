#!/usr/bin/env bash
set -euo pipefail

ENDPOINT="https://socai.io/v1/events"
DEFAULT_BIN="target/release/socai"
BIN="${SOCAI_BIN:-$DEFAULT_BIN}"
DATASET="${AXIOM_DATASET:-socai-cli-prod}"
LIVE=0
SKIP_BUILD=0
SKIP_AXIOM=0
KEEP_HOME=0
RUN_ID="$(date +%Y%m%d%H%M%S)"
QUERY="socai-telemetry-smoke-${RUN_ID}"

usage() {
  cat <<'USAGE'
Usage: scripts/telemetry-release-smoke-test.sh [options]

Verifies release-candidate CLI telemetry properties without embedding secrets.
By default it performs static binary checks only. Use --live to run CLI commands
against the production first-party telemetry proxy and verify Axiom rows.

Options:
  --bin PATH        CLI binary to test (default: target/release/socai, or SOCAI_BIN)
  --dataset NAME   Axiom dataset for live verification (default: socai-cli-prod)
  --live           Run live CLI telemetry checks in an isolated SOCAI_HOME
  --skip-build     Do not run cargo build when testing target/release/socai
  --skip-axiom     In --live mode, skip Axiom CLI queries and check local JSONL only
  --keep-home      Keep the temporary SOCAI_HOME after live checks for inspection
  --query TEXT     Base smoke-test query text for live checks
  -h, --help       Show this help

Prerequisites for --live:
  - Browser/session prerequisites needed by socai commands in this environment.
  - axiom CLI authenticated for the target dataset, unless --skip-axiom is used.
  - Network access to https://socai.io/v1/events.
USAGE
}

log() {
  printf '[telemetry-smoke] %s\n' "$*" >&2
}

fail() {
  printf '[telemetry-smoke] ERROR: %s\n' "$*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin)
      BIN="${2:-}"
      [[ -n "$BIN" ]] || fail "--bin requires a path"
      shift 2
      ;;
    --dataset)
      DATASET="${2:-}"
      [[ -n "$DATASET" ]] || fail "--dataset requires a name"
      shift 2
      ;;
    --live)
      LIVE=1
      shift
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --skip-axiom)
      SKIP_AXIOM=1
      shift
      ;;
    --keep-home)
      KEEP_HOME=1
      shift
      ;;
    --query)
      QUERY="${2:-}"
      [[ -n "$QUERY" ]] || fail "--query requires text"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

if [[ "$BIN" == "$DEFAULT_BIN" && "$SKIP_BUILD" -eq 0 ]]; then
  log "building release candidate binary with cargo build -p socai-cli --release"
  cargo build -p socai-cli --release
fi

[[ -x "$BIN" ]] || fail "CLI binary is not executable: $BIN"
command -v strings >/dev/null 2>&1 || fail "strings command is required"

log "checking fixed telemetry endpoint is present in binary strings"
strings "$BIN" | grep -F "$ENDPOINT" >/dev/null || fail "fixed endpoint not found in binary strings"

log "checking binary strings for Axiom-specific client secrets/config"
if strings "$BIN" | grep -E 'AXIOM_(TOKEN|DATASET|URL|ORG_ID)|api\.axiom\.co' >/dev/null; then
  strings "$BIN" | grep -E 'AXIOM_(TOKEN|DATASET|URL|ORG_ID)|api\.axiom\.co' >&2 || true
  fail "Axiom-specific secret/config string found in client binary"
fi

log "static binary checks passed"

if [[ "$LIVE" -eq 0 ]]; then
  log "live telemetry checks skipped; rerun with --live for production proxy/Axiom verification"
  exit 0
fi

command -v python3 >/dev/null 2>&1 || fail "python3 is required for --live"
if [[ "$SKIP_AXIOM" -eq 0 ]]; then
  command -v axiom >/dev/null 2>&1 || fail "axiom CLI is required for --live unless --skip-axiom is set"
fi

SMOKE_HOME="$(mktemp -d "${TMPDIR:-/tmp}/socai-telemetry-smoke.XXXXXX")"
EVENTS_FILE="$SMOKE_HOME/telemetry/events.jsonl"
export SOCAI_HOME="$SMOKE_HOME"

cleanup() {
  "$BIN" stop >/dev/null 2>&1 || true
  if [[ "$KEEP_HOME" -eq 1 ]]; then
    log "kept SOCAI_HOME=$SMOKE_HOME"
  else
    rm -rf "$SMOKE_HOME"
  fi
}
trap cleanup EXIT

json_count() {
  python3 - "$EVENTS_FILE" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
if not path.exists():
    print(0)
else:
    print(sum(1 for line in path.read_text().splitlines() if line.strip()))
PY
}

wait_for_count() {
  local expected="$1"
  local deadline=$((SECONDS + 30))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    if [[ "$(json_count)" -ge "$expected" ]]; then
      return 0
    fi
    sleep 1
  done
  fail "timed out waiting for at least $expected local telemetry row(s) in $EVENTS_FILE"
}

last_request_id() {
  python3 - "$EVENTS_FILE" <<'PY'
import json, pathlib, sys
rows = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines() if line.strip()]
props = rows[-1].get('properties', {})
print(props.get('request_id', ''))
PY
}

validate_last_default_query() {
  local expected_query="$1"
  python3 - "$EVENTS_FILE" "$expected_query" <<'PY'
import json, pathlib, sys
rows = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines() if line.strip()]
props = rows[-1].get('properties', {})
expected = sys.argv[2]
assert props.get('query_text_enabled') is True, props
assert props.get('query_text') == expected, props
assert props.get('request_id'), props
print(props['request_id'])
PY
}

validate_last_redacted_query() {
  python3 - "$EVENTS_FILE" <<'PY'
import json, pathlib, sys
rows = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines() if line.strip()]
props = rows[-1].get('properties', {})
assert props.get('query_text_enabled') is False, props
assert 'query_text' not in props, props
assert props.get('request_id'), props
print(props['request_id'])
PY
}

run_search_notes() {
  local label="$1"
  local query="$2"
  shift 2
  log "running ${label}: socai search_notes ${query}"
  set +e
  "$@" "$BIN" search_notes "$query" >/tmp/socai-telemetry-smoke-${label}.out 2>/tmp/socai-telemetry-smoke-${label}.err
  local status=$?
  set -e
  if [[ "$status" -ne 0 ]]; then
    log "${label} command exited with status $status; continuing because failed commands should still emit telemetry"
    log "stderr saved to /tmp/socai-telemetry-smoke-${label}.err"
  fi
}

query_axiom_by_request_id() {
  local request_id="$1"
  log "waiting for proxy flush before querying Axiom for request_id=$request_id"
  sleep 8
  local output
  output="$(axiom query "['${DATASET}'] | where request_id == '${request_id}' | limit 1" --start-time=-30m --format=json --no-spinner)"
  [[ -n "$output" ]] || fail "no Axiom row found for request_id=$request_id"
  printf '%s\n' "$output"
}

assert_axiom_no_query_text() {
  local axiom_row="$1"
  local forbidden_query="$2"
  AXIOM_ROW="$axiom_row" python3 - "$forbidden_query" <<'PY'
import json, os, sys
forbidden = sys.argv[1]
text = os.environ.get('AXIOM_ROW', '').strip()
assert text, 'empty Axiom output'
row = json.loads(text.splitlines()[0])
assert row.get('query_text') in (None, ''), row
assert row.get('query_text') != forbidden, row
PY
}

assert_axiom_no_query_row() {
  local query="$1"
  log "checking Axiom has no opt-out row for query_text=$query"
  sleep 8
  local output
  output="$(axiom query "['${DATASET}'] | where query_text == '${query}' | limit 1" --start-time=-30m --format=json --no-spinner)"
  [[ -z "$output" ]] || fail "unexpected Axiom row found for opt-out query_text=$query: $output"
}

log "using isolated SOCAI_HOME=$SOCAI_HOME"
"$BIN" stop >/dev/null 2>&1 || true

DEFAULT_QUERY="${QUERY}-default"
run_search_notes "default" "$DEFAULT_QUERY" env
wait_for_count 1
DEFAULT_REQUEST_ID="$(validate_last_default_query "$DEFAULT_QUERY")"
log "default telemetry request_id=$DEFAULT_REQUEST_ID"
if [[ "$SKIP_AXIOM" -eq 0 ]]; then
  query_axiom_by_request_id "$DEFAULT_REQUEST_ID" >/tmp/socai-telemetry-smoke-default.axiom.json
fi

BEFORE_OPTOUT="$(json_count)"
OPTOUT_QUERY="${QUERY}-optout"
run_search_notes "optout" "$OPTOUT_QUERY" env SOCAI_TELEMETRY=off
sleep 5
AFTER_OPTOUT="$(json_count)"
[[ "$AFTER_OPTOUT" == "$BEFORE_OPTOUT" ]] || fail "SOCAI_TELEMETRY=off wrote a local telemetry row"
if [[ "$SKIP_AXIOM" -eq 0 ]]; then
  assert_axiom_no_query_row "$OPTOUT_QUERY"
fi
log "opt-out check passed"

REDACTED_QUERY="${QUERY}-redacted"
EXPECTED_COUNT=$((AFTER_OPTOUT + 1))
run_search_notes "redacted" "$REDACTED_QUERY" env SOCAI_TELEMETRY_QUERY_TEXT=off
wait_for_count "$EXPECTED_COUNT"
REDACTED_REQUEST_ID="$(validate_last_redacted_query)"
log "redacted telemetry request_id=$REDACTED_REQUEST_ID"
if [[ "$SKIP_AXIOM" -eq 0 ]]; then
  REDACTED_AXIOM_ROW="$(query_axiom_by_request_id "$REDACTED_REQUEST_ID")"
  assert_axiom_no_query_text "$REDACTED_AXIOM_ROW" "$REDACTED_QUERY"
fi

log "live telemetry smoke checks passed"
