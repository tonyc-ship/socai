# socai CLI telemetry dashboard

This runbook defines the first Axiom dashboard for the finalized socai CLI
telemetry trace schema. The core implementation landed in PR #63 and emits one
sanitized trace per top-level CLI tool command.

## Status

The Axiom datasets and production proxy already exist:

- Production dataset: `socai-cli-prod`
- Development dataset: `socai-cli-dev`
- Production ingest endpoint: `https://socai.io/v1/events`
- Proxy source: `site/api/telemetry.js`

This repository does **not** store Axiom tokens. Tokens live only in the Axiom
account and server-side Vercel environment variables.

The installed Axiom CLI currently supports query/dataset/auth management but not
dashboard creation, so create the dashboard manually in Axiom using the panel
queries below. The queries were smoke-tested against `socai-cli-prod` with the
Axiom CLI on 2026-06-05.

## Dashboard setup

Recommended dashboard name:

```text
socai CLI telemetry
```

Recommended global time range:

```text
Last 7 days
```

Use `Last 24 hours` while validating production deploys or release candidates.

## Final trace schema summary

Each accepted CLI trace represents one top-level daemon command:

- `search_notes` -> `tool_name=search_notes`
- `topic_scan` -> `tool_name=topic_scan`
- `extract_note` -> `tool_name=read_note`

Useful fields for dashboard panels:

| Category | Fields |
| --- | --- |
| Identity/correlation | `install_id`, `session_id`, `request_id` |
| App/device | `app`, `source`, `app_version`, `platform`, `os_version`, `os_kernel_version`, `memory_total_mb`, `cpu_count`, `terminal_app`, `parent_process` |
| Command/tool | `command`, `tool_name`, `site` |
| Query | `query_text_enabled`, `query_text`, `query_len` |
| Explicit parameters | `metadata.*`, especially `metadata.num_notes`, `metadata.tab`, `metadata.debug_snapshot` |
| Outcome | `duration_ms`, `ok`, `error`, `result_ok`, `has_run_dir` |
| Result metrics | `cards_count`, `search_cards_count`, `selected_cards_count`, `notes_count`, `notes_skipped_count` |

Axiom may still show old columns such as `event`, `arch`, `created_at_ms`,
`client_created_at_ms`, `received_at_ms`, or `query_redacted` as `null` because
older rows created those fields. New proxy-forwarded rows should not populate
those fields.

## Panels

### 1. Trace volume over time

Purpose: confirm the proxy is receiving traffic and spot ingest gaps.

Suggested visualization: line chart.

```apl
['socai-cli-prod']
| summarize traces=count() by bin(_time, 1h)
| order by _time asc
```

For sparse periods, switch `1h` to `1d`.

### 2. Command/tool usage

Purpose: show which CLI commands are used most.

Suggested visualization: table or bar chart.

```apl
['socai-cli-prod']
| where isnotempty(command)
| summarize traces=count() by command, tool_name
| order by traces desc
| limit 25
```

### 3. Top query text

Purpose: understand user search intent. This only includes rows where query text
was enabled.

Suggested visualization: table.

```apl
['socai-cli-prod']
| where query_text_enabled == true and isnotempty(query_text)
| summarize traces=count() by query_text
| order by traces desc
| limit 25
```

Privacy note: query text is included by default, but users can redact it with
`SOCAI_TELEMETRY_QUERY_TEXT=off`.

### 4. Query redaction rate

Purpose: track how often users redact query text.

Suggested visualization: stacked bar or pie chart.

```apl
['socai-cli-prod']
| where isnotnull(query_text_enabled)
| summarize traces=count() by query_text_enabled
| order by traces desc
```

### 5. Failure rate by command/tool

Purpose: find commands that fail disproportionately.

Suggested visualization: table with conditional formatting.

```apl
['socai-cli-prod']
| where isnotempty(command)
| summarize failures=countif(ok == false), traces=count() by command, tool_name
| extend failure_rate_pct=round(100.0 * todouble(failures) / todouble(traces), 2)
| order by failure_rate_pct desc
| limit 25
```

### 6. Duration percentiles by command/tool

Purpose: monitor latency and detect slowdowns.

Suggested visualization: table or line chart per command.

```apl
['socai-cli-prod']
| where isnotempty(command) and isnotnull(duration_ms)
| summarize
    p50_ms=percentile(duration_ms, 50),
    p95_ms=percentile(duration_ms, 95),
    p99_ms=percentile(duration_ms, 99)
  by command, tool_name
| order by p95_ms desc
| limit 25
```

### 7. Result counts and skipped notes

Purpose: measure output volume and topic-scan quality.

Suggested visualization: table.

```apl
['socai-cli-prod']
| extend
    cards=column_ifexists('cards_count', long(null)),
    search_cards=column_ifexists('search_cards_count', long(null)),
    selected=column_ifexists('selected_cards_count', long(null)),
    notes=column_ifexists('notes_count', long(null)),
    skipped=column_ifexists('notes_skipped_count', long(null))
| where isnotempty(command)
| summarize
    avg_cards=avg(cards),
    avg_search_cards=avg(search_cards),
    avg_selected=avg(selected),
    avg_notes=avg(notes),
    avg_skipped=avg(skipped),
    total_notes=sum(notes),
    total_skipped=sum(skipped)
  by command, tool_name
| order by total_notes desc
| limit 25
```

`column_ifexists` keeps this panel valid when a metric has not appeared in the
dataset yet.

### 8. Terminal, platform, and OS breakdown

Purpose: understand where the CLI is used and detect environment-specific
problems.

Suggested visualization: table or stacked bar chart.

```apl
['socai-cli-prod']
| summarize traces=count() by terminal_app, platform, os_version, os_kernel_version
| order by traces desc
| limit 25
```

### 9. Explicit `--num-notes` usage

Purpose: understand how often users override the default topic-scan note count.
Defaults are intentionally omitted; this panel only shows explicitly supplied
values.

Suggested visualization: histogram or table.

```apl
['socai-cli-prod']
| where isnotnull(['metadata.num_notes'])
| extend requested_num_notes=toint(['metadata.num_notes'])
| summarize traces=count() by tostring(requested_num_notes)
| order by traces desc
| limit 25
```

Optional companion stat:

```apl
['socai-cli-prod']
| where isnotnull(['metadata.num_notes'])
| extend requested_num_notes=toint(['metadata.num_notes'])
| summarize
    avg_num_notes=avg(requested_num_notes),
    p50_num_notes=percentile(requested_num_notes, 50),
    p95_num_notes=percentile(requested_num_notes, 95)
```

### 10. Explicit `--tab` usage

Purpose: see which topic tabs users request explicitly.

Suggested visualization: table.

```apl
['socai-cli-prod']
| where isnotempty(['metadata.tab'])
| summarize traces=count() by tab=['metadata.tab']
| order by traces desc
| limit 25
```

### 11. Explicit `--debug-snapshot` usage

Purpose: track opt-in debug snapshot usage without treating the default `false`
value as user intent.

Suggested visualization: stat or table.

```apl
['socai-cli-prod']
| extend debug_snapshot=column_ifexists('metadata.debug_snapshot', bool(null))
| summarize debug_snapshot_traces=countif(debug_snapshot == true), traces=count() by command, tool_name
| order by debug_snapshot_traces desc
| limit 25
```

## Optional monitors

Create monitors only if we want production alerting now. Otherwise, keep these as
manual dashboard checks.

### Failure-rate monitor

Suggested schedule: every 15 minutes over the last hour.

Alert when any command has at least 5 traces and failure rate is >= 20%.

```apl
['socai-cli-prod']
| where isnotempty(command)
| summarize failures=countif(ok == false), traces=count() by command, tool_name
| extend failure_rate_pct=100.0 * todouble(failures) / todouble(traces)
| where traces >= 5 and failure_rate_pct >= 20
| order by failure_rate_pct desc
```

### Latency monitor

Suggested schedule: every 15 minutes over the last hour.

Alert when any command has at least 5 traces and p95 duration exceeds 120 seconds.

```apl
['socai-cli-prod']
| where isnotempty(command) and isnotnull(duration_ms)
| summarize p95_ms=percentile(duration_ms, 95), traces=count() by command, tool_name
| where traces >= 5 and p95_ms > 120000
| order by p95_ms desc
```

### Ingest silence monitor

Suggested schedule: every 30 minutes over the last 24 hours.

Use this only after regular production traffic exists; otherwise it will be noisy.

```apl
['socai-cli-prod']
| summarize traces=count()
| where traces == 0
```

## CLI validation commands

Use these commands to validate key panel queries from a maintainer machine with
Axiom CLI auth. Do not pass or commit tokens in command history.

```bash
axiom query "['socai-cli-prod'] | summarize traces=count() by bin(_time, 1h) | order by _time asc | limit 5" --start-time=-24h --format=json --no-spinner
axiom query "['socai-cli-prod'] | where isnotempty(command) | summarize traces=count() by command, tool_name | order by traces desc | limit 10" --start-time=-7d --format=json --no-spinner
axiom query "['socai-cli-prod'] | where query_text_enabled == true and isnotempty(query_text) | summarize traces=count() by query_text | order by traces desc | limit 10" --start-time=-7d --format=json --no-spinner
axiom query "['socai-cli-prod'] | where isnotempty(command) and isnotnull(duration_ms) | summarize p50_ms=percentile(duration_ms, 50), p95_ms=percentile(duration_ms, 95), p99_ms=percentile(duration_ms, 99) by command, tool_name | order by p95_ms desc | limit 10" --start-time=-7d --format=json --no-spinner
```

## Dashboard creation checklist

1. Open Axiom and select dataset `socai-cli-prod`.
2. Create dashboard `socai CLI telemetry`.
3. Add the panels above with the recommended visualization types.
4. Save the dashboard and copy its URL into the maintainer telemetry runbook.
5. Optionally configure the monitors above once there is enough baseline traffic.
