# Telemetry release smoke test

Use this runbook before publishing a CLI release, and again after installing the
published artifact, to verify that release telemetry still uses the first-party
proxy and does not expose Axiom credentials.

The smoke test intentionally does **not** require an Axiom token in the CLI or in
repo files. Axiom access is only needed by the maintainer running verification,
through their local `axiom` CLI login or server-side proxy configuration.

## What this verifies

- The release-candidate binary contains the fixed proxy endpoint:
  `https://socai.io/v1/events`.
- The client binary does not contain Axiom endpoint/env/token strings.
- The test starts from a stopped daemon so old daemon processes do not emit old
  telemetry shapes.
- A normal CLI command emits one tool trace through the production proxy.
- `SOCAI_TELEMETRY=off` emits no telemetry row for that command.
- `SOCAI_TELEMETRY_QUERY_TEXT=off` keeps telemetry on but omits `query_text`.

## Prerequisites

- A release-candidate `socai` binary, or a checkout that can build one with
  `cargo build -p socai-cli --release`.
- Browser/session prerequisites needed by the command being tested.
- Network access to `https://socai.io/v1/events`.
- For remote Axiom checks: authenticated `axiom` CLI access to `socai-cli-prod`.
  Do not put Axiom tokens in the repo or pass them to the `socai` binary.

## Automated helper

From the repository root:

```bash
scripts/telemetry-release-smoke-test.sh
```

By default the helper performs static binary checks only. It builds
`target/release/socai` unless `--skip-build` is passed, then verifies:

1. the binary contains `https://socai.io/v1/events`; and
2. the binary does not contain Axiom-specific client strings such as
   `AXIOM_TOKEN`, `AXIOM_DATASET`, `AXIOM_URL`, `AXIOM_ORG_ID`, or
   `api.axiom.co`.

To run live telemetry checks against the production proxy and Axiom:

```bash
scripts/telemetry-release-smoke-test.sh --live
```

Useful options:

```bash
scripts/telemetry-release-smoke-test.sh --bin /path/to/socai --skip-build
scripts/telemetry-release-smoke-test.sh --live --dataset socai-cli-prod
scripts/telemetry-release-smoke-test.sh --live --skip-axiom   # local JSONL checks only
scripts/telemetry-release-smoke-test.sh --live --keep-home    # keep temp SOCAI_HOME
```

The helper uses an isolated temporary `SOCAI_HOME`, runs `socai stop` before the
checks, and writes no secrets. In `--live` mode it runs `search_notes` three
times with unique smoke-test query strings:

1. default telemetry/query behavior;
2. `SOCAI_TELEMETRY=off`; and
3. `SOCAI_TELEMETRY_QUERY_TEXT=off`.

Command failures are reported but do not immediately abort the telemetry check,
because failed tool commands should still emit a failure trace. If no local
telemetry row appears, the helper fails.

## Manual checks

If the helper is not suitable for the release environment, use these steps.

### 1. Build or install the release candidate

```bash
cargo build -p socai-cli --release
bin=target/release/socai
```

For a packaged artifact, set `bin` to the installed binary path.

### 2. Verify fixed endpoint and no Axiom client config

```bash
strings "$bin" | grep -F 'https://socai.io/v1/events'

if strings "$bin" | grep -E 'AXIOM_(TOKEN|DATASET|URL|ORG_ID)|api\.axiom\.co'; then
  echo 'unexpected Axiom client string found' >&2
  exit 1
fi
```

Expected result: the first command prints the first-party proxy endpoint; the
second command prints nothing and exits through the non-error path.

### 3. Start from a clean daemon

Use an isolated home so this smoke test does not affect the maintainer's normal
socai daemon or telemetry identity:

```bash
export SOCAI_HOME="$(mktemp -d "${TMPDIR:-/tmp}/socai-telemetry-smoke.XXXXXX")"
"$bin" stop || true
```

### 4. Verify default telemetry emits one trace

```bash
query="socai-telemetry-smoke-$(date +%s)-default"
"$bin" search_notes "$query" || true
```

Inspect the local JSONL buffer:

```bash
python3 - "$SOCAI_HOME/telemetry/events.jsonl" "$query" <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1])
query = sys.argv[2]
rows = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
assert len(rows) == 1, len(rows)
props = rows[-1]['properties']
assert props['query_text_enabled'] is True, props
assert props['query_text'] == query, props
assert props['request_id'], props
print(props['request_id'])
PY
```

Use the printed `request_id` to confirm production Axiom ingestion:

```bash
request_id='<printed-request-id>'
axiom query "['socai-cli-prod'] | where request_id == '$request_id' | limit 1" \
  --start-time=-30m \
  --format=json \
  --no-spinner
```

Expected result: one Axiom row for that `request_id`. Existing Axiom datasets may
still show removed legacy columns as `null`; that does not mean the current
client/proxy sent those fields.

### 5. Verify full opt-out

```bash
before=$(wc -l < "$SOCAI_HOME/telemetry/events.jsonl")
query="socai-telemetry-smoke-$(date +%s)-optout"
SOCAI_TELEMETRY=off "$bin" search_notes "$query" || true
sleep 5
after=$(wc -l < "$SOCAI_HOME/telemetry/events.jsonl")
test "$before" = "$after"
```

Optionally confirm no production row exists for the unique query text:

```bash
axiom query "['socai-cli-prod'] | where query_text == '$query' | limit 1" \
  --start-time=-30m \
  --format=json \
  --no-spinner
```

Expected result: no output.

### 6. Verify query redaction

```bash
query="socai-telemetry-smoke-$(date +%s)-redacted"
SOCAI_TELEMETRY_QUERY_TEXT=off "$bin" search_notes "$query" || true
```

Inspect the newest local row:

```bash
python3 - "$SOCAI_HOME/telemetry/events.jsonl" <<'PY'
import json, pathlib, sys
rows = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines() if line.strip()]
props = rows[-1]['properties']
assert props['query_text_enabled'] is False, props
assert 'query_text' not in props, props
assert props['request_id'], props
print(props['request_id'])
PY
```

Then query Axiom by the printed `request_id` and confirm `query_text` is absent
or `null`.

### 7. Clean up

```bash
"$bin" stop || true
rm -rf "$SOCAI_HOME"
unset SOCAI_HOME
```

## Release notes reminder

After installing an updated CLI, users with an existing background daemon should
restart it so the new telemetry shape is used:

```bash
socai stop
```
