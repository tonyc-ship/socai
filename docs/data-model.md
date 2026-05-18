# socai data model

This document describes the persisted data that backs agent runs and the desktop
app task history. Keep this current when changing run artifacts, task state, or
history replay.

## Guiding rule

`~/.socai/runs/...` is the source of truth for task results.

The desktop app may keep an index in `~/.socai/app/tasks.json`, but it should not
persist duplicate task result payloads such as final answers or event timelines.
The app can hydrate those fields from the run directory when serving the UI.

## Locations

| Data | Default path | Override | Owner |
| --- | --- | --- | --- |
| Core run artifacts | `~/.socai/runs/agent_<timestamp>_<task-slug>/` | `SOCAI_RUNS_DIR` | `socai-core` |
| Desktop task index | `~/.socai/app/tasks.json` | `SOCAI_HOME` (`$SOCAI_HOME/app/tasks.json`) | Tauri app |

Do not add a second persistent task-result store under `~/.socai/app/`. If the
UI needs result data after restart, derive it from the task's `run_dir`.

## Identifiers

### `task_id`

Created by the desktop app task registry before the agent starts.

Example:

```txt
task-1790000000000-1
```

Used for desktop UI state: task list rows, selected task, cancellation, and
mapping an app task to a core run.

### `run_id`

Created inside `socai-core` when the agent loop starts.

Example:

```txt
20260518-142233-123456
```

Used by the core agent run and persisted in run artifacts / task index once the
run exists.

### `run_dir`

Created before starting the core run and passed into `socai-core`.

Example:

```txt
~/.socai/runs/agent_20260518_142233_find_xhs_coffee_notes
```

This directory is the source of truth for final answer and timeline replay.

`task_id` and `run_id` are intentionally different. `task_id` is an app/workflow
identifier; `run_id` is a core agent-run identifier.

## Core run directory

Each agent run writes files under `run_dir`.

| Path | Purpose | Notes |
| --- | --- | --- |
| `report.md` | Final answer shown by the desktop app | Source of truth for completed task output. May include appended artifact markdown. |
| `reasoning_log.jsonl` | Append-only debug/event log | Primary source for historical timeline replay. |
| `conversation.json` | Final system prompt and message history snapshot | Useful for debugging/model replay. |
| `agent_log.json` | Run summary | Includes task, model, turns, token counts, paths to run files. |
| `tool_results/<turn>_<seq>_<tool>.json` | Full tool result bodies | `reasoning_log.jsonl` only stores compact summaries. |
| `run_state/events.jsonl` | Compact run-state timeline | Fallback source for historical timeline replay. |
| `run_state/working_memory.md` | Rendered working memory | Used by the agent context/memory path. |
| `run_state/artifacts.json` | Artifact registry | Screenshots/media/other artifacts saved by tools. |
| `run_state/evidence.json` | Extracted evidence records | Populated from entity-like tool artifacts. |
| `run_state/plan.json` | Current plan state | Updated by plan tools when used. |

### `reasoning_log.jsonl`

One JSON object per line. Current important event types include:

- `task_start`
- `llm_response`
- `tool_call_start`
- `tool_result`
- `api_error`
- `turn_end`
- `task_end`

The desktop app replays timeline rows from this file first. If it is absent or
empty, the app falls back to `run_state/events.jsonl`.

## Desktop task index: `tasks.json`

`tasks.json` is an app index, not a task result store. It is an array of task
snapshots with fields like:

```json
{
  "task_id": "task-1790000000000-1",
  "task": "find popular xhs coffee notes",
  "model": "claude-sonnet-4-5-20250929",
  "status": "completed",
  "created_at": 1790000000000,
  "started_at": 1790000000200,
  "finished_at": 1790000060000,
  "run_id": "20260518-142233-123456",
  "run_dir": "/Users/alice/.socai/runs/agent_20260518_142233_find_popular_xhs_coffee_notes",
  "target_id": null,
  "error": null,
  "turns": 4,
  "input_tokens": 12345,
  "output_tokens": 2345
}
```

Rules:

- `final_text` is not persisted in `tasks.json`.
- If old `tasks.json` files contain `final_text`, the app ignores it on load and
  omits it the next time the index is written.
- API responses may still include `final_text`, but it is hydrated from
  `<run_dir>/report.md`.
- `error` is status metadata for failed/interrupted tasks; it is not a final
  answer.
- Queued/running tasks are marked `interrupted` on app startup because their
  in-process runner and browser tab no longer exist.

## Desktop timeline state

Live event rows are held in frontend memory while a task is running. They are
streamed over Tauri's `agent_task:event` event as:

```json
{
  "task_id": "task-1790000000000-1",
  "kind": "tool_call",
  "text": "search_notes({\"query\":\"...\"})",
  "snapshot": null,
  "sequence": 0,
  "created_at": 1790000005000
}
```

After restart or webview reload, the app does not read a separate app event
cache. Instead, `agent_task_events(task_id)` reconstructs rows from the task's
`run_dir`:

1. `<run_dir>/reasoning_log.jsonl`
2. fallback: `<run_dir>/run_state/events.jsonl`
3. terminal status metadata from `tasks.json` when applicable

This keeps timeline and final answer data rooted in the run directory.

## Realtime event stream vs `reasoning_log.jsonl`

`reasoning_log.jsonl` is not an exact byte-for-byte copy of what the user sees
in realtime.

The live UI receives two classes of events:

1. App lifecycle/status events emitted by Tauri: `queued`, `running`, `tab`,
   `completed`, `failed`, `cancelled`, `interrupted`.
2. Core agent events emitted by `socai-core`: `started`, `turn`, `assistant`,
   `reasoning`, `tool_call`, `tool_result`, `tool_error`, `api_error`, `done`.

`reasoning_log.jsonl` is a persistent core debug log with enough structured data
to replay the important run timeline, but some live rows are represented
differently or not present.

| UI row | Live source | `reasoning_log.jsonl` source | Replay behavior |
| --- | --- | --- | --- |
| `queued` | Tauri | Not logged | Not replayed after restart. |
| `running` | Tauri | Not logged | Not replayed after restart. |
| `tab` | Tauri | Not logged | Not replayed after restart. |
| `started` | `AgentEvent::Started` | `task_start` | Replayed. |
| `turn` | `AgentEvent::Turn` | No standalone row; inferred from `turn` fields | Reconstructed. |
| `assistant` | `AgentEvent::AssistantText` | `llm_response.text` | Replayed. |
| `reasoning` | `AgentEvent::Reasoning` | Not currently logged | Not replayed today. |
| `tool_call` | `AgentEvent::ToolCall` | `tool_call_start` | Replayed. |
| `tool_result` / `tool_error` | `AgentEvent::ToolResult` | `tool_result` | Replayed from summary/error. |
| `api_error` | `AgentEvent::ApiError` | `api_error` | Replayed. |
| `done` | `AgentEvent::Done` | `task_end` | Replayed. |
| terminal app status | Tauri snapshot update | `tasks.json` status metadata | Reconstructed from task status. |

If we ever need perfect historical replay, the better foundation would be to
make the core agent event stream serializable and persist that stream in the run
directory. It should still live under `run_dir`, not under the app index.
